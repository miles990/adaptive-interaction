#!/usr/bin/env bash
# Reproducible compile check for the ESP32 reference firmware (arduino-cli).
#
#   ./firmware/esp32-companion/compile.sh            # Serial + Wi-Fi/MQTT build
#   ./firmware/esp32-companion/compile.sh --ble      # + NimBLE (ENABLE_BLE=1)
#   ./firmware/esp32-companion/compile.sh --setup    # install core + libraries first
#
# This only proves the sketch compiles against esp32:esp32 3.x and the pinned
# libraries. It is NOT a hardware acceptance test — flashing a real board and
# running the README test table is still required for that.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FQBN="${FQBN:-esp32:esp32:esp32}"
OUT="${OUT:-$(mktemp -d "${TMPDIR:-/tmp}/esp32-companion-build.XXXXXX")}"
BLE=0; SETUP=0
for a in "$@"; do
  case "$a" in
    --ble) BLE=1 ;;
    --setup) SETUP=1 ;;
    *) echo "unknown flag: $a" >&2; exit 2 ;;
  esac
done
command -v arduino-cli >/dev/null || { echo "arduino-cli not found (brew install arduino-cli)" >&2; exit 1; }

if [ "$SETUP" = 1 ]; then
  arduino-cli config init --overwrite >/dev/null
  arduino-cli config add board_manager.additional_urls \
    https://raw.githubusercontent.com/espressif/arduino-esp32/gh-pages/package_esp32_index.json
  arduino-cli core update-index
  arduino-cli core install esp32:esp32
  arduino-cli lib install "ArduinoJson" "PubSubClient" "DHT sensor library" "ESP32Servo" "NimBLE-Arduino"
fi

# Sketch folder: copy the .ino + a config.h derived from the example (never
# touches a real config.h with your Wi-Fi password).
SKETCH="$OUT/esp32-companion"
mkdir -p "$SKETCH"
cp "$HERE/esp32-companion.ino" "$SKETCH/"
cp "$HERE/config.h.example" "$SKETCH/config.h"
[ "$BLE" = 1 ] && echo '#define ENABLE_BLE 1' >> "$SKETCH/config.h"

EXTRA=()
# Apple Silicon without Rosetta: the bundled x86_64 ctags cannot run.
BUNDLED="$(ls -d "$HOME"/Library/Arduino15/packages/builtin/tools/ctags/*/ 2>/dev/null | head -1 || true)"
if [ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] && [ -n "$BUNDLED" ] \
   && ! "$BUNDLED/ctags" --version >/dev/null 2>&1; then
  echo "note: bundled ctags is x86_64-only and Rosetta is absent — using tools/ctags-shim (Universal Ctags)" >&2
  EXTRA+=(--build-property "runtime.tools.ctags.path=$HERE/tools/ctags-shim")
fi

echo "== arduino-cli compile --fqbn $FQBN (ENABLE_BLE=$BLE) =="
arduino-cli compile --fqbn "$FQBN" --warnings all --build-path "$OUT/build" "${EXTRA[@]}" "$SKETCH"
echo "== artifacts =="
ls -la "$OUT/build"/*.bin
