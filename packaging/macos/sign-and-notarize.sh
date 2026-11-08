#!/usr/bin/env bash
# 对 macOS .app 做 Developer ID 签名、Hardened Runtime 加固、DMG 生成与公证。
#
# 用法: sign-and-notarize.sh <Transom.app> <输出.dmg> <自更新.zip>
#
# 设计原理（Why）：公证对象是 DMG，而 Gatekeeper 最终执行的是签名后的
# .app 与其中主程序。脚本先按由内到外的顺序签名，再基于已签名 .app 生成
# DMG，提交 notarytool 并 staple 票据，最后从同一主程序生成自更新 zip，
# 确保更新包与手动分发包使用完全相同的二进制。

set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "用法: $0 <Transom.app> <输出.dmg> <自更新.zip>" >&2
    exit 2
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP_PATH="$1"
DMG_PATH="$2"
ZIP_PATH="$3"
PRODUCT_NAME="${PRODUCT_NAME:-Transom}"
EXECUTABLE_NAME="${EXECUTABLE_NAME:-transom}"
ENTITLEMENTS_PATH="${ENTITLEMENTS_PATH:-${ROOT_DIR}/packaging/macos/Transom.entitlements}"

require_env() {
    local name="$1"
    if [[ -z "${!name:-}" ]]; then
        echo "缺少 macOS 签名或公证环境变量: ${name}" >&2
        exit 1
    fi
}

for name in \
    APPLE_CERTIFICATE_BASE64 \
    APPLE_CERTIFICATE_PASSWORD \
    APPLE_ID \
    APPLE_APP_SPECIFIC_PASSWORD \
    APPLE_TEAM_ID; do
    require_env "$name"
done

if [[ ! -d "$APP_PATH" ]]; then
    echo "待签名 .app 不存在: $APP_PATH" >&2
    exit 1
fi
if [[ ! -f "$ENTITLEMENTS_PATH" ]]; then
    echo "entitlements 文件不存在: $ENTITLEMENTS_PATH" >&2
    exit 1
fi

APP_PATH="$(cd "$(dirname "$APP_PATH")" && pwd)/$(basename "$APP_PATH")"
MAIN_EXECUTABLE="${APP_PATH}/Contents/MacOS/${EXECUTABLE_NAME}"
if [[ ! -x "$MAIN_EXECUTABLE" ]]; then
    echo "主程序不存在或不可执行: $MAIN_EXECUTABLE" >&2
    exit 1
fi

WORK_DIR="$(mktemp -d "${RUNNER_TEMP:-/tmp}/transom-macos-sign.XXXXXX")"
KEYCHAIN_PATH="${WORK_DIR}/transom-signing.keychain-db"
KEYCHAIN_PASSWORD="$(openssl rand -hex 24)"
CERT_PATH="${WORK_DIR}/developer-id-application.p12"

EXISTING_KEYCHAINS=()
while IFS= read -r keychain; do
    keychain="${keychain//\"/}"
    keychain="${keychain#"${keychain%%[![:space:]]*}"}"
    keychain="${keychain%"${keychain##*[![:space:]]}"}"
    if [[ -n "$keychain" ]]; then
        EXISTING_KEYCHAINS+=("$keychain")
    fi
done < <(security list-keychains -d user)

cleanup() {
    local status=$?
    security delete-keychain "$KEYCHAIN_PATH" >/dev/null 2>&1 || true
    if [[ ${#EXISTING_KEYCHAINS[@]} -gt 0 ]]; then
        security list-keychains -d user -s "${EXISTING_KEYCHAINS[@]}" >/dev/null 2>&1 || true
    fi
    rm -rf "$WORK_DIR"
    exit "$status"
}
trap cleanup EXIT

printf '%s' "$APPLE_CERTIFICATE_BASE64" |
    openssl base64 -d -A -out "$CERT_PATH"

security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security import "$CERT_PATH" \
    -k "$KEYCHAIN_PATH" \
    -P "$APPLE_CERTIFICATE_PASSWORD" \
    -T /usr/bin/codesign
security set-key-partition-list \
    -S apple-tool:,apple:,codesign: \
    -s \
    -k "$KEYCHAIN_PASSWORD" \
    "$KEYCHAIN_PATH" >/dev/null

if [[ ${#EXISTING_KEYCHAINS[@]} -gt 0 ]]; then
    security list-keychains -d user -s "$KEYCHAIN_PATH" "${EXISTING_KEYCHAINS[@]}"
else
    security list-keychains -d user -s "$KEYCHAIN_PATH"
fi

SIGNING_IDENTITY="$(
    security find-identity -v -p codesigning "$KEYCHAIN_PATH" |
        awk -F '"' '/"Developer ID Application:/ { print $2; exit }'
)"
if [[ -z "$SIGNING_IDENTITY" ]]; then
    echo "临时钥匙串中未找到 Developer ID Application 证书" >&2
    exit 1
fi

echo "签署 macOS 主程序与应用包: $SIGNING_IDENTITY"
codesign \
    --force \
    --options runtime \
    --timestamp \
    --entitlements "$ENTITLEMENTS_PATH" \
    --keychain "$KEYCHAIN_PATH" \
    --sign "$SIGNING_IDENTITY" \
    "$MAIN_EXECUTABLE"
codesign \
    --force \
    --options runtime \
    --timestamp \
    --entitlements "$ENTITLEMENTS_PATH" \
    --keychain "$KEYCHAIN_PATH" \
    --sign "$SIGNING_IDENTITY" \
    "$APP_PATH"
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
CODESIGN_INFO="$(codesign --display --verbose=4 "$APP_PATH" 2>&1)"
if ! grep -q 'flags=.*runtime' <<<"$CODESIGN_INFO"; then
    echo "应用包未启用 Hardened Runtime" >&2
    exit 1
fi

STAGING_DIR="${WORK_DIR}/dmg"
mkdir -p "$STAGING_DIR" "$(dirname "$DMG_PATH")" "$(dirname "$ZIP_PATH")"
ditto "$APP_PATH" "${STAGING_DIR}/${PRODUCT_NAME}.app"
ln -s /Applications "${STAGING_DIR}/Applications"
hdiutil create \
    -volname "$PRODUCT_NAME" \
    -srcfolder "$STAGING_DIR" \
    -ov \
    -format UDZO \
    "$DMG_PATH" >/dev/null
hdiutil verify "$DMG_PATH" >/dev/null

codesign \
    --force \
    --timestamp \
    --keychain "$KEYCHAIN_PATH" \
    --sign "$SIGNING_IDENTITY" \
    "$DMG_PATH"
codesign --verify --verbose=2 "$DMG_PATH"

NOTARY_LOG="${WORK_DIR}/notarytool.log"
set +e
NOTARY_OUTPUT="$(
    xcrun notarytool submit "$DMG_PATH" \
        --apple-id "$APPLE_ID" \
        --password "$APPLE_APP_SPECIFIC_PASSWORD" \
        --team-id "$APPLE_TEAM_ID" \
        --wait 2>&1
)"
NOTARY_STATUS=$?
set -e
printf '%s\n' "$NOTARY_OUTPUT" | tee "$NOTARY_LOG"

SUBMISSION_ID="$(
    printf '%s\n' "$NOTARY_OUTPUT" |
        sed -n 's/^[[:space:]]*id: //p' |
        head -n 1
)"
if [[ $NOTARY_STATUS -ne 0 ]] || ! grep -q 'status: Accepted' "$NOTARY_LOG"; then
    if [[ -n "$SUBMISSION_ID" ]]; then
        xcrun notarytool log "$SUBMISSION_ID" \
            --apple-id "$APPLE_ID" \
            --password "$APPLE_APP_SPECIFIC_PASSWORD" \
            --team-id "$APPLE_TEAM_ID" || true
    fi
    echo "Apple 公证未通过，DMG: $DMG_PATH" >&2
    exit 1
fi

xcrun stapler staple "$DMG_PATH"
xcrun stapler validate "$DMG_PATH"
codesign --verify --verbose=2 "$DMG_PATH"

zip -j -q "$ZIP_PATH" "$MAIN_EXECUTABLE"
VERIFY_DIR="${WORK_DIR}/update-verify"
mkdir -p "$VERIFY_DIR"
unzip -q "$ZIP_PATH" -d "$VERIFY_DIR"
codesign --verify --strict --verbose=2 "${VERIFY_DIR}/${EXECUTABLE_NAME}"

echo "macOS 签名、公证、stapling 与自更新 zip 验证完成"
