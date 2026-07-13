# restrike-web

The web frontend for Restrike (`docs/design/renderer-ui-shell.md` D-UI6):
wasm-bindgen + canvas 2D. One page, one canvas, keyboard only -- the D8
proof that nothing in `gbx-engine` blocks, even across a JS event loop, not
a product. Game data is supplied by the user at runtime via a directory
picker (never bundled, D10).

## Build

Requires the `wasm32-unknown-unknown` target and `wasm-bindgen-cli` pinned
to the **exact same version** as the `wasm-bindgen` crate in `Cargo.lock`
(check with `grep -A1 'name = "wasm-bindgen"' ../../Cargo.lock`; at the time
of writing that's `0.2.126`):

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126

cargo build -p restrike-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir frontends/web/pkg \
  target/wasm32-unknown-unknown/release/restrike_web.wasm
```

(run from the repo root; adjust `--out-dir`/the `.wasm` path if run from
elsewhere.)

## Serve

Any static file server works -- the page is plain HTML/JS/wasm, no
server-side logic. From the repo root:

```sh
python3 -m http.server 8080 --directory frontends/web
```

Then open `http://localhost:8080/` and pick your CotAB data directory.

CI only `cargo check`s this crate against `wasm32-unknown-unknown` (the
gating check per D-UI6); the `wasm-bindgen` packaging step above is a local
concern.
