#!/bin/bash
set -euo pipefail

# CI only. Supply certificates through the protected release environment.
: "${RUNNER_TEMP:?Run this on a GitHub-hosted macOS release runner}"
: "${MACOS_CERTIFICATE_P12:?Set the release environment certificate secret}"
: "${MACOS_CERTIFICATE_PASSWORD:?Set the certificate password secret}"
: "${APPLE_ID:?Set the notarization account secret}"
: "${APPLE_TEAM_ID:?Set the Apple team secret}"
: "${APPLE_APP_PASSWORD:?Set the notarization password secret}"

keychain="$RUNNER_TEMP/webskiff-release.keychain-db"
certificate="$(mktemp "$RUNNER_TEMP/webskiff-certificate.XXXXXX")"
trap 'rm -f "$certificate"' EXIT
password="$(openssl rand -hex 32)"
printf '::add-mask::%s\n' "$password"
printf '%s' "$MACOS_CERTIFICATE_P12" | base64 --decode > "$certificate"
chmod 600 "$certificate"
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$certificate" -P "$MACOS_CERTIFICATE_PASSWORD" \
  -T /usr/bin/codesign -t cert -f pkcs12 -k "$keychain"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain"
security list-keychains -d user -s "$keychain"
xcrun notarytool store-credentials webskiff-notary \
  --keychain "$keychain" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
  --password "$APPLE_APP_PASSWORD"
security default-keychain -d user -s "$keychain"
