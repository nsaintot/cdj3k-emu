#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# appcast.sh VERSION TAG BUILD DIR NOTES_HTML - write DIR/appcast.xml, the
# Sparkle feed the macOS app reads from the latest release, for the one
# macOS .dmg in DIR.
#
# BUILD is the CD run number, which the bundle carries as CFBundleVersion and
# Sparkle compares against sparkle:version. The .dmg is signed with the update
# key (sign.sh), whose public half Info.plist carries as SUPublicEDKey.
set -euo pipefail
version="$1"
tag="$2"
build="$3"
dir="$4"
notes_html="$5"
: "${GITHUB_REPOSITORY:?}"
script_dir="$(cd "$(dirname "$0")" && pwd)"

shopt -s nullglob
dmgs=("$dir"/CDJ3K-Emulator-"$version"-macos-*.dmg)
[[ ${#dmgs[@]} -eq 1 ]] || { echo "::error::expected one macOS .dmg in $dir, found ${#dmgs[@]}" >&2; exit 1; }
dmg="${dmgs[0]}"
name=$(basename "$dmg")

signature=$("$script_dir/sign.sh" "$dmg")
length=$(wc -c < "$dmg" | tr -d ' ')
url="https://github.com/$GITHUB_REPOSITORY/releases/download/$tag/$name"
published=$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')
# A CDATA section cannot hold its own terminator.
notes=$(sed 's/]]>/]]]]><![CDATA[>/g' "$notes_html")

cat > "$dir/appcast.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>CDJ3K Emulator</title>
    <link>https://github.com/$GITHUB_REPOSITORY/releases</link>
    <item>
      <title>Version $version</title>
      <pubDate>$published</pubDate>
      <sparkle:version>$build</sparkle:version>
      <sparkle:shortVersionString>$version</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>15.0</sparkle:minimumSystemVersion>
      <sparkle:fullReleaseNotesLink>https://github.com/$GITHUB_REPOSITORY/releases/tag/$tag</sparkle:fullReleaseNotesLink>
      <description><![CDATA[$notes]]></description>
      <enclosure url="$url" length="$length" type="application/octet-stream" sparkle:edSignature="$signature"/>
    </item>
  </channel>
</rss>
XML
cat "$dir/appcast.xml"
