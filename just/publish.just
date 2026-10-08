# Publish the sidemirror crate
[group('publish')]
publish_crate args="":
  cargo publish --locked --features smb {{ args }}
