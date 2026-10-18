#!/usr/bin/env bash
# Flash one half of the Cornix LP with the UF2 files in firmware/.
#
# Usage: ./flash.sh left|right [--yes]
#   --yes     skip the "press Enter to continue" confirmation after the Bluetooth warning
#
# To flash a GitHub Actions build, download its UF2 files into firmware/ first:
#   gh run download <run-id> -n rmk-cornix-uf2 -D firmware/
# firmware/ may also hold recovery images; preserve files other than the two build outputs.
#
# Connect the half over USB and put it into bootloader mode (double-tap the reset button, or for the
# left half Vial > Security > Reboot to bootloader). left = central (rmk-cornix-central.uf2),
# right = peripheral (rmk-cornix-peripheral.uf2).
#
# The script waits for macOS to mount the bootloader drive (a volume named "cornix", in any letter
# case, with INFO_UF2.TXT), copies the file onto it, and waits for the reboot into the new firmware.
# If no drive appears in bootloader mode, check whether ESET device control blocks the mount.
#
# Bluetooth: `mise run build` and CI images carry a new BUILD_HASH, which makes RMK erase its storage
# and bonds; the warning printed before flashing says what to do. The script cannot tell which kind
# of image it is flashing, so it does not touch the host's Bluetooth settings.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

half="" assume_yes=0
for a in "$@"; do
  case $a in
    left|right) half=$a ;;
    --yes|-y) assume_yes=1 ;;
    *) echo "usage: $0 left|right [--yes]" >&2; exit 2 ;;
  esac
done
[[ $half == left || $half == right ]] || { echo "usage: $0 left|right [--yes]" >&2; exit 2; }

# --- Bluetooth warning ---
cat >&2 <<'WARN'
────────────────────────────────────────────────────────────────────────
  If this image comes from `mise run build` or from CI, flashing it
  erases all Bluetooth bonds on this half. Each host keeps its old bond
  and cannot reconnect until you forget the keyboard and pair again:
    macOS: System Settings > Bluetooth > Cornix > Forget This Device
  The two halves pair with each other again on their own.
  Using `cargo make uf2` alone does not guarantee bond retention:
  the image's BUILD_HASH must match the installed firmware.
────────────────────────────────────────────────────────────────────────
WARN

if [[ $assume_yes -eq 0 && -t 0 ]]; then
  read -r -p "Press Enter to continue flashing the $half half, or Ctrl-C to abort... " _
fi

case $half in
  left) file=firmware/rmk-cornix-central.uf2 ;;
  right) file=firmware/rmk-cornix-peripheral.uf2 ;;
esac
[[ -f $file ]] || { echo "missing $file (build it, or download a CI build: see the header)" >&2; exit 1; }

# Only a volume named cornix with INFO_UF2.TXT is accepted. With both halves connected,
# this does not identify which half is mounted; connect only the half being flashed.
find_cornix_volume() {
  local v
  for v in /Volumes/*/; do
    [[ ${v%/} == */[Cc][Oo][Rr][Nn][Ii][Xx] && -f "$v/INFO_UF2.TXT" ]] && { echo "${v%/}"; return 0; }
  done
  return 1
}

WAIT_TIMEOUT=600
echo "Put the $half half into bootloader mode with USB connected (double-tap reset). Waiting up to ${WAIT_TIMEOUT}s..."
vol=""
for ((i = 0; i < WAIT_TIMEOUT; i++)); do
  vol=$(find_cornix_volume) && break
  sleep 1
done
if [[ -z $vol ]]; then
  echo "error: no Cornix bootloader drive appeared within ${WAIT_TIMEOUT}s (if the keyboard is in bootloader mode, check ESET's device control)" >&2
  exit 1
fi

# The bootloader reboots as soon as the last block is written, so cp may report an I/O error at
# the very end. Disappearance is treated as the expected bootloader reboot, not as read-back
# verification of the image. If the drive remains after REBOOT_TIMEOUT seconds, the script fails.
REBOOT_TIMEOUT=30
echo "found $vol: $(head -n 1 "$vol/INFO_UF2.TXT")"
cp "$file" "$vol/" || echo "(cp reported an error; the bootloader usually reboots before cp returns)"
echo "copied $file; waiting for the drive to disappear..."
for ((i = 0; i < REBOOT_TIMEOUT; i++)); do
  [[ -d $vol ]] || break
  sleep 1
done
if [[ -d $vol ]]; then
  echo "error: $vol is still mounted after ${REBOOT_TIMEOUT}s; the bootloader did not reboot, so the write probably failed" >&2
  exit 1
fi

echo "done: $half half flashed (the bootloader rebooted into the new image)"
