#!/bin/bash
set -euo pipefail

if [[ $# != 3 || "${CI:-}" != true || "$(uname -m)" != arm64 || -z "${RUNNER_TEMP:-}" ]]; then
  echo 'Usage on a fresh macOS ARM64 CI runner: test-source-brew.sh FORMULA COMMIT VERSION' >&2
  exit 1
fi

formula="$1"
commit="$2"
version="$3"
[[ -f "$formula" && "$commit" =~ ^[0-9a-f]{40}$ ]]

export HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_ANALYTICS=1 HOMEBREW_NO_INSTALL_CLEANUP=1
export HOMEBREW_CURLRC="$RUNNER_TEMP/webskiff-curlrc"
printf 'header = "User-Agent: OpenAI File Downloader, XaiImageApiFetch/1.0"\n' > "$HOMEBREW_CURLRC"

if brew list --formula | grep -Fxq webskiff; then
  echo 'Webskiff is already installed; a fresh runner is required.' >&2
  exit 1
fi

brew install engineersamuel/tap/webskiff
previous_version="$(brew list --versions webskiff | cut -d' ' -f2)"
tap="$(brew --repo engineersamuel/tap)"
cp "$formula" "$tap/Formula/webskiff.rb"
git -C "$tap" config user.name 'Webskiff test'
git -C "$tap" config user.email 'webskiff-test@users.noreply.github.com'
git -C "$tap" add Formula/webskiff.rb
if ! git -C "$tap" diff --cached --quiet; then
  git -C "$tap" commit -m 'Test candidate formula locally'
fi

user_home="$(mktemp -d "$RUNNER_TEMP/webskiff-user.XXXXXX")"
mkdir -p "$user_home/.config/webskiff" "$user_home/.cache/zweb/profile"
printf '# user configuration\n' > "$user_home/.config/webskiff/config.toml"
printf 'user profile\n' > "$user_home/.cache/zweb/profile/keep"

if [[ "$version" = "$previous_version" ]]; then
  echo "Testing replacement source installation at $version"
  brew reinstall --build-from-source engineersamuel/tap/webskiff
else
  echo "Testing Homebrew upgrade: $previous_version -> $version"
  brew upgrade --build-from-source engineersamuel/tap/webskiff
fi

installed="$(webskiff --version)"
printf '%s\n' "$installed"
[[ "$installed" = "webskiff $version "* && "$installed" = *"$commit"* ]]
brew style --formula engineersamuel/tap/webskiff
brew audit --strict --formula engineersamuel/tap/webskiff
brew test engineersamuel/tap/webskiff
HOME="$user_home" webskiff --selftest | grep 'ALL PASSED'

source_dir="$(mktemp -d "$RUNNER_TEMP/webskiff-source.XXXXXX")"
archive="$(brew --cache engineersamuel/tap/webskiff)"
tar -xzf "$archive" -C "$source_dir" --strip-components=1
for mode in --check-only --config-check --enhanced-check; do
  uv run --no-project python -I "$source_dir/bench/keyboard.py" "$(brew --prefix)/bin/webskiff" "$mode"
done

HOME="$user_home" brew uninstall engineersamuel/tap/webskiff
test "$(cat "$user_home/.config/webskiff/config.toml")" = '# user configuration'
test "$(cat "$user_home/.cache/zweb/profile/keep")" = 'user profile'
test ! -e "$(brew --prefix)/bin/webskiff"
