#!/usr/bin/env bash
set -euo pipefail

REPOSITORY="${MICRO_REPOSITORY:-rmonvfer/micro}"
INSTALL_DIR="${MICRO_INSTALL_DIR:-$HOME/.local/bin}"
DIST_DIR="${MICRO_DIST_DIR:-$HOME/.local/share/micro/dist}"
DOWNLOAD_ROOT="https://github.com/${REPOSITORY}/releases/download"
TEMP_DIR=""
STAGED_DIST=""
GITHUB_AUTH_TOKEN=""

die() {
    echo "error: $1" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

download() {
    local arguments=(--fail --silent --show-error --location --retry 3 --retry-delay 1)
    if [ -n "$GITHUB_AUTH_TOKEN" ]; then
        arguments+=(--header "Authorization: Bearer ${GITHUB_AUTH_TOKEN}")
    fi
    curl "${arguments[@]}" "$@"
}

github_token() {
    if [ -n "${MICRO_GITHUB_TOKEN:-}" ]; then
        printf '%s' "$MICRO_GITHUB_TOKEN"
    elif [ -n "${GITHUB_TOKEN:-}" ]; then
        printf '%s' "$GITHUB_TOKEN"
    elif [ -n "${GH_TOKEN:-}" ]; then
        printf '%s' "$GH_TOKEN"
    elif command -v gh >/dev/null 2>&1; then
        gh auth token 2>/dev/null || true
    fi
}

download_release_asset() {
    local tag="$1" asset="$2"
    if [ -n "$GITHUB_AUTH_TOKEN" ]; then
        require_command gh
        GH_TOKEN="$GITHUB_AUTH_TOKEN" gh release download "$tag" \
            --repo "$REPOSITORY" \
            --pattern "$asset" \
            --dir "$TEMP_DIR"
    else
        download --output "${TEMP_DIR}/${asset}" "${DOWNLOAD_ROOT}/${tag}/${asset}"
    fi
}

cleanup() {
    [ -z "$STAGED_DIST" ] || rm -rf "$STAGED_DIST"
    [ -z "$TEMP_DIR" ] || rm -rf "$TEMP_DIR"
}

platform() {
    local os arch
    case "$(uname -s)" in
        Darwin) os="darwin" ;;
        Linux) os="linux" ;;
        *) die "unsupported OS: $(uname -s)" ;;
    esac
    case "$(uname -m)" in
        x86_64|amd64) arch="x86_64" ;;
        arm64|aarch64) arch="arm64" ;;
        *) die "unsupported architecture: $(uname -m)" ;;
    esac
    if [ "$os" = "darwin" ] && [ "$arch" = "x86_64" ]; then
        die "Intel macOS is not supported"
    fi
    echo "${os}-${arch}"
}

version() {
    if [ -n "${MICRO_VERSION:-}" ]; then
        echo "$MICRO_VERSION"
        return
    fi
    local latest
    latest=$(download "https://api.github.com/repos/${REPOSITORY}/releases/latest" | sed -nE 's/.*"tag_name": *"([^"]+)".*/\1/p' | head -n 1)
    [ -n "$latest" ] || die "failed to fetch the latest release"
    echo "$latest"
}

verify_version() {
    [[ "$1" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]] || die "invalid release version: $1"
}

checksum() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        die "sha256sum or shasum is required to verify the download"
    fi
}

main() {
    local target tag asset expected actual version_dir
    require_command curl
    require_command mktemp
    require_command tar
    GITHUB_AUTH_TOKEN=$(github_token)
    target=$(platform)
    tag=$(version)
    verify_version "$tag"
    asset="micro-${target}.tar.gz"
    TEMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/micro-install.XXXXXXXX")
    trap cleanup EXIT

    echo "Installing micro ${tag} for ${target}..."
    download_release_asset "$tag" "$asset" || die "failed to download ${asset}"
    download_release_asset "$tag" "checksums-sha256.txt" || die "failed to download checksums"
    expected=$(awk -v asset="$asset" '$2 == asset || $2 == "*" asset { print $1 }' "${TEMP_DIR}/checksums-sha256.txt")
    [ -n "$expected" ] || die "release checksums do not contain ${asset}"
    [[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]] || die "invalid SHA-256 checksum"
    actual=$(checksum "${TEMP_DIR}/${asset}")
    [ "$actual" = "$expected" ] || die "checksum verification failed for ${asset}"

    mkdir -p "$DIST_DIR"
    STAGED_DIST=$(mktemp -d "${DIST_DIR}/.install.XXXXXXXX")
    tar -xzf "${TEMP_DIR}/${asset}" -C "$STAGED_DIST" --strip-components 1 || die "failed to extract ${asset}"
    [ -x "${STAGED_DIST}/bin/micro" ] || die "release archive does not contain bin/micro"
    version_dir="${DIST_DIR}/${tag#v}"
    rm -rf "$version_dir"
    mv "$STAGED_DIST" "$version_dir"
    STAGED_DIST=""
    mkdir -p "$INSTALL_DIR"
    ln -sfn "${version_dir}/bin/micro" "${INSTALL_DIR}/micro"

    echo "Installed micro to ${version_dir}"
    echo "Linked ${INSTALL_DIR}/micro"
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        echo "Add this to your shell profile: export PATH=\"${INSTALL_DIR}:\$PATH\""
    fi
}

main
