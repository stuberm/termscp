//! The generic remotefs metadata drops WebDAV ETags. Read the strong validator
//! separately using the same DAV transport/authentication as the file adapter.
use std::path::{Component, Path};

use remotefs_webdav::dav_xml_client::{AsyncDavClient, Auth, Resource};
use url::Url;

use super::filesystem::VersionProbe;
use crate::filetransfer::params::WebDAVProtocolParams;

pub(super) fn probe(params: &WebDAVProtocolParams) -> Option<VersionProbe> {
    let base = Url::parse(&params.uri).ok()?;
    let client = AsyncDavClient::reqwest(Auth::basic(&params.username, &params.password)).ok()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    Some(Box::new(move |path| {
        let Some(url) = resource_url(&base, path) else {
            return Ok(None);
        };
        // An unsupported/failed validator request is a cache miss, not permission
        // to reuse an old result. The normal adapter still checks stat/read errors.
        let resource = runtime.block_on(client.stat(url.as_str())).ok().flatten();
        Ok(resource.and_then(|resource| strong_version(resource, &url)))
    }))
}

fn strong_version(resource: Resource, requested: &Url) -> Option<String> {
    // A depth-zero response must identify this exact resource, not a parent or
    // an unrelated entry returned first in a malformed multistatus response.
    if resource.is_collection
        || requested.join(&resource.href.to_string()).ok().as_ref() != Some(requested)
    {
        return None;
    }
    // The DAV library's ETag parser deliberately accepts unquoted/malformed
    // tags. Cache validation must be stricter: inspect the original property.
    use remotefs_webdav::dav_xml_client::dav_xml::Value;
    let Value::Map(properties) = Value::from(resource.prop) else {
        return None;
    };
    let mut etags = properties.iter_ordered().filter(|(name, _)| {
        name.namespace.as_deref() == Some("DAV:") && name.local_name == "getetag"
    });
    let (_, value) = etags.next()?;
    if etags.next().is_some() {
        return None;
    }
    let raw = value.as_str().ok()?.trim();
    let opaque = raw.strip_prefix('"')?.strip_suffix('"')?;
    if !opaque
        .bytes()
        .all(|byte| byte == 0x21 || (0x23..=0x7e).contains(&byte) || byte >= 0x80)
    {
        return None;
    }
    Some(format!("webdav-strong-etag:{raw}"))
}

fn resource_url(base: &Url, path: &Path) -> Option<Url> {
    if !path.has_root() {
        return None;
    }
    let mut url = base.clone();
    {
        let mut segments = url.path_segments_mut().ok()?;
        segments.pop_if_empty();
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    segments.push(name.to_str()?);
                }
                _ => return None,
            }
        }
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_the_exact_authenticated_webdav_resource_over_http() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (requests, observed) = mpsc::channel();
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("validator request did not arrive: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap_or("0")
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = "<D:multistatus xmlns:D=\"DAV:\"><D:response><D:href>/dav/folder/a%20b</D:href><D:propstat><D:prop><D:getetag>\"v1\"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>";
            write!(stream, "HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            requests.send(String::from_utf8(bytes).unwrap()).unwrap();
        });
        let mut probe = probe(&WebDAVProtocolParams {
            uri: format!("http://{address}/dav"),
            username: "user".into(),
            password: "pass".into(),
        })
        .unwrap();
        assert_eq!(
            probe(Path::new("/folder/a b")).unwrap().as_deref(),
            Some("webdav-strong-etag:\"v1\"")
        );
        let request = observed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(request.starts_with("PROPFIND /dav/folder/a%20b HTTP/1.1\r\n"));
        let headers = request.to_ascii_lowercase();
        assert!(headers.contains("depth: 0\r\n"));
        assert!(headers.contains("authorization: basic dxnlcjpwyxnz\r\n"));
    }

    #[test]
    fn weak_missing_and_failed_etag_properties_are_not_validators() {
        use remotefs_webdav::dav_xml_client::dav_xml::FromXml;
        use remotefs_webdav::dav_xml_client::dav_xml::elements::Response;
        for (property, status, expected) in [
            (
                "<D:getetag>\"v1\"</D:getetag>",
                "200 OK",
                Some("webdav-strong-etag:\"v1\""),
            ),
            ("<D:getetag>W/\"v1\"</D:getetag>", "200 OK", None),
            ("<D:getetag>w/\"v1\"</D:getetag>", "200 OK", None),
            ("<D:getetag>unquoted</D:getetag>", "200 OK", None),
            ("<D:getetag>\"bad tag\"</D:getetag>", "200 OK", None),
            ("<D:getetag>\"bad\"tag\"</D:getetag>", "200 OK", None),
            ("<D:getetag/>", "200 OK", None),
            ("", "200 OK", None),
            ("<D:getetag>\"v1\"</D:getetag>", "404 Not Found", None),
        ] {
            let xml = format!(
                "<D:response xmlns:D=\"DAV:\"><D:href>/file</D:href><D:propstat><D:prop>{property}</D:prop><D:status>HTTP/1.1 {status}</D:status></D:propstat></D:response>"
            );
            let response = Response::from_xml(xml.into_bytes()).unwrap();
            assert_eq!(
                strong_version(
                    Resource::from_response(&response).unwrap(),
                    &Url::parse("https://host/file").unwrap()
                )
                .as_deref(),
                expected
            );
        }
    }

    #[test]
    fn a_version_from_another_resource_or_origin_cannot_validate_this_path() {
        use remotefs_webdav::dav_xml_client::dav_xml::FromXml;
        use remotefs_webdav::dav_xml_client::dav_xml::elements::Response;
        for href in ["/parent", "https://other/file", "/file?other"] {
            let xml = format!(
                "<D:response xmlns:D=\"DAV:\"><D:href>{href}</D:href><D:propstat><D:prop><D:getetag>\"v1\"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
            );
            let response = Response::from_xml(xml.into_bytes()).unwrap();
            assert!(
                strong_version(
                    Resource::from_response(&response).unwrap(),
                    &Url::parse("https://host/file").unwrap()
                )
                .is_none(),
                "unexpected validator for {href}"
            );
        }
    }

    #[test]
    fn preserves_share_prefix_and_encodes_names_not_separators() {
        let base = Url::parse("https://host/dav/share/").unwrap();
        assert_eq!(
            resource_url(&base, Path::new("/folder/a b?#%.txt"))
                .unwrap()
                .as_str(),
            "https://host/dav/share/folder/a%20b%3F%23%25.txt"
        );
        assert!(resource_url(&base, Path::new("relative")).is_none());
        assert!(resource_url(&base, Path::new("/../escape")).is_none());
    }
}
