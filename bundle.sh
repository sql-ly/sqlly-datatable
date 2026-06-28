#!/bin/sh
BIN=target/debug/gpui-grid
APP=target/debug/GpuiGrid.app

i=0
while [ "$i" -lt 600 ]; do
  if [ -f "$BIN" ]; then
    s1=$(stat -f%z "$BIN" 2>/dev/null)
    sleep 0.1
    s2=$(stat -f%z "$BIN" 2>/dev/null)
    if [ -n "$s1" ] && [ "$s1" = "$s2" ] && [ "$s1" -gt 0 ]; then
      break
    fi
  fi
  sleep 0.1
  i=$((i + 1))
done

[ -f "$BIN" ] || exit 0

mkdir -p "$APP/Contents/MacOS"
cp "$BIN" "$APP/Contents/MacOS/GpuiGrid"
cat > "$APP/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>GpuiGrid</string>
<key>CFBundleIdentifier</key><string>com.local.gpui-grid</string>
<key>CFBundleName</key><string>GpuiGrid</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
EOF
