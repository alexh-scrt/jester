# Example Configs

- `minimal.jester.toml` – TLS listener on `:443` that forwards `example.com` traffic to `http://127.0.0.1:8080`.

Tips:
1. Duplicate the file and adjust certificate/key paths for your environment (`certs/dev.crt`, `certs/dev.key`, etc.).
2. Update `hosts`/`targets` to point at a local upstream service; `python -m http.server 8080` works for smoke tests.
3. Use environment placeholders when keeping secrets out of version control, e.g.:
   ```toml
   [listeners.tls]
   cert = "${CERT_PATH:certs/dev.crt}"
   key  = "${KEY_PATH:certs/dev.key}"
   ```
4. Run `cargo run -p jester-cli -- config validate path/to/config.toml` after every edit to catch mistakes early.
