#!/bin/sh
# Build the sqlly-datatable sample app as a WebAssembly web app and package
# everything needed to run it into a single zip.
#
# Usage:
#   ./build-wasm.sh            # release build (default)
#   ./build-wasm.sh --debug    # unoptimized build
#
# Output:
#   dist/web/                                    the runnable site
#   dist/sqlly-datatable-web-v<version>.zip      the same site, zipped
#
# Requirements: rustup; everything else (the nightly toolchain, the wasm32
# target, and `wasm-bindgen-cli` at the exact version the workspace's
# `wasm-bindgen` crate is locked to) is installed automatically.
#
# The wasm build uses NIGHTLY rust: gpui's web backend (`gpui_web`) enables
# `parking_lot/nightly` and depends on `wasm_thread`, which needs the
# unstable `stdarch_wasm_atomic_wait` feature on wasm32. This mirrors the
# upstream `gpui-component` story-web setup. Native builds stay on stable.
#
# Run from anywhere; the script always resolves paths relative to itself.

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SCRIPT_DIR"

PROFILE=release
CARGO_FLAGS="--release"

for arg in "$@"; do
    case "$arg" in
        --debug)
            PROFILE=debug
            CARGO_FLAGS=""
            ;;
        *)
            echo "unknown option: $arg" >&2
            echo "usage: $0 [--debug]" >&2
            exit 2
            ;;
    esac
done

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

echo "==> ensuring nightly toolchain with wasm32-unknown-unknown target..."
rustup toolchain install nightly --profile minimal
rustup target add wasm32-unknown-unknown --toolchain nightly

# The wasm-bindgen CLI must match the crate version pinned in Cargo.lock.
WANT_BINDGEN=$(awk '/^name = "wasm-bindgen"$/{getline; sub(/version = "/, ""); sub(/"$/, ""); print; exit}' Cargo.lock)
if [ -z "$WANT_BINDGEN" ]; then
    echo "error: wasm-bindgen not found in Cargo.lock" >&2
    exit 1
fi
HAVE_BINDGEN=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [ "$HAVE_BINDGEN" != "$WANT_BINDGEN" ]; then
    echo "==> installing wasm-bindgen-cli $WANT_BINDGEN (found: ${HAVE_BINDGEN:-none})..."
    cargo install wasm-bindgen-cli --version "$WANT_BINDGEN" --locked
fi

echo "==> building sample for wasm32-unknown-unknown ($PROFILE, nightly)..."
cargo +nightly build -p sqlly-datatable-sample --lib --target wasm32-unknown-unknown --locked $CARGO_FLAGS

WASM="target/wasm32-unknown-unknown/$PROFILE/sqlly_datatable_sample.wasm"
if [ ! -f "$WASM" ]; then
    echo "error: wasm artifact not found at $WASM" >&2
    exit 1
fi

echo "==> generating JS bindings..."
rm -rf dist/web
mkdir -p dist/web
wasm-bindgen "$WASM" --out-dir dist/web --target web --no-typescript

# Bundle the lucide icon SVGs next to the site: on wasm the
# `gpui-component-assets` source fetches `{endpoint}/assets/icons/*.svg`
# instead of embedding them in the binary (the sample passes `.` as the
# endpoint, so they resolve relative to the page).
echo "==> bundling icon assets..."
ASSETS_DIR=$(cargo metadata --format-version 1 --locked 2>/dev/null | python3 -c '
import json, sys, os
meta = json.load(sys.stdin)
for pkg in meta["packages"]:
    if pkg["name"] == "gpui-component-assets":
        print(os.path.join(os.path.dirname(pkg["manifest_path"]), "assets"))
        break
')
if [ -z "$ASSETS_DIR" ] || [ ! -d "$ASSETS_DIR/icons" ]; then
    echo "error: gpui-component-assets icons not found (looked in: ${ASSETS_DIR:-<unresolved>})" >&2
    exit 1
fi
mkdir -p dist/web/assets/icons
cp "$ASSETS_DIR"/icons/*.svg dist/web/assets/icons/

echo "==> writing index.html..."
cat > dist/web/index.html <<'HTML'
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>sqlly-datatable — web sample</title>
    <style>
      * { margin: 0; padding: 0; box-sizing: border-box; }
      html, body { width: 100%; height: 100%; overflow: hidden; background: #14181a; }
      /* gpui creates and appends its own canvas; make it fill the page. */
      canvas { display: block; width: 100vw; height: 100vh; }
      #loading {
        position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%);
        text-align: center; color: #9aa4a8;
        font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
      }
      .spinner {
        border: 4px solid rgba(255, 255, 255, 0.1); border-left-color: #4db6ac;
        border-radius: 50%; width: 40px; height: 40px;
        animation: spin 1s linear infinite; margin: 0 auto 16px;
      }
      @keyframes spin { to { transform: rotate(360deg); } }
      .error { color: #ef5350; padding: 20px; max-width: 40em; }
    </style>
  </head>
  <body>
    <div id="loading">
      <div class="spinner"></div>
      <p>Loading sqlly-datatable…</p>
    </div>
    <script type="module">
      const loading = document.getElementById('loading');
      try {
        const wasm = await import('./sqlly_datatable_sample.js');
        await wasm.default();
        wasm.run();
        // The app opens its window (canvas) once WebGPU is ready; drop the
        // spinner when the canvas shows up (or give up after 15s).
        const started = Date.now();
        const poll = setInterval(() => {
          if (document.querySelector('canvas')) {
            loading.remove();
            clearInterval(poll);
          } else if (Date.now() - started > 15000) {
            loading.innerHTML =
              '<div class="error"><h2>Timed out waiting for WebGPU</h2>' +
              '<p>This demo needs a browser with WebGPU enabled ' +
              '(Chrome/Edge 113+, recent Safari or Firefox). ' +
              'Check the console for details.</p></div>';
            clearInterval(poll);
          }
        }, 100);
      } catch (error) {
        console.error('Failed to start:', error);
        loading.innerHTML =
          '<div class="error"><h2>Failed to load the application</h2><p>' +
          (error && error.message ? error.message : error) + '</p></div>';
      }
    </script>
  </body>
</html>
HTML

cat > dist/web/README.txt <<TXT
sqlly-datatable web sample v$VERSION
====================================

A WebAssembly build of the sqlly-datatable sample app (GPUI + gpui-component
running in the browser via WebGPU).

To run: serve this directory over HTTP and open it in a WebGPU-capable
browser (Chrome/Edge 113+, or recent Safari/Firefox with WebGPU enabled).
ES modules and wasm do not load from file:// URLs, so a server is required:

    python3 -m http.server 8080
    # then open http://localhost:8080/

Files:
    index.html                     entry page (loads the module, shows status)
    sqlly_datatable_sample.js      wasm-bindgen JS glue
    sqlly_datatable_sample_bg.wasm the application
    assets/icons/                  lucide icon SVGs (fetched on demand)
TXT

ZIP="sqlly-datatable-web-v$VERSION.zip"
echo "==> packaging dist/$ZIP..."
rm -f "dist/$ZIP"
(cd dist/web && zip -q -r "../$ZIP" .)

echo "==> done"
ls -lh "dist/$ZIP"
echo "    unzip anywhere and serve:  python3 -m http.server  (see README.txt)"
