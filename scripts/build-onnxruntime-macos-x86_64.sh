#!/usr/bin/env bash
# ort-sys 2.0.0-rc.15 downloads ONNX Runtime 1.31, but its prebuilt table has no
# x86_64-apple-darwin entry and Microsoft stopped publishing osx-x86_64 archives
# with 1.30. An Intel macOS runner therefore builds the dylib from source and
# hands the directory to ORT_LIB_LOCATION=<dir> ORT_PREFER_DYNAMIC_LINK=1.
set -euo pipefail

version=${ORT_VERSION:-1.31.0}
src=${ORT_SRC_DIR:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/onnxruntime-src}
deploy=${MACOSX_DEPLOYMENT_TARGET:-13.4}
out=${1:?usage: scripts/build-onnxruntime-macos-x86_64.sh <out-dir>}

[[ $(uname -s) == Darwin ]] || {
  echo "error: macOS only, this produces a Mach-O dylib (uname -s = $(uname -s))" >&2
  exit 1
}
mkdir -p "$out" && out=$(cd "$out" && pwd)

if [[ ! -d $src ]]; then
  git clone --depth 1 --branch "v${version}" --recursive --shallow-submodules \
    https://github.com/microsoft/onnxruntime.git "$src"
fi
src=$(cd "$src" && pwd)   # absolute, so `built` below is not relative to the build dir

# CMake 4.x on the runner rejects onnxruntime's older cmake_minimum_required;
# CMAKE_POLICY_VERSION_MINIMUM restores the pre-4.0 compatibility floor.
# --skip_tests only skips running them, so the test binaries are turned off too.
cd "$src"
./build.sh --config Release --build_shared_lib --parallel --skip_tests \
  --skip_submodule_sync --compile_no_warning_as_error --no_telemetry \
  --osx_arch x86_64 \
  --cmake_extra_defines CMAKE_OSX_ARCHITECTURES=x86_64 \
  CMAKE_OSX_DEPLOYMENT_TARGET="$deploy" CMAKE_POLICY_VERSION_MINIMUM=3.5 \
  onnxruntime_BUILD_UNIT_TESTS=OFF

built="$src/build/MacOS/Release/libonnxruntime.${version}.dylib"
[[ -f $built ]] || { echo "error: $built missing — build.sh produced no shared library" >&2; exit 1; }

lib="$out/libonnxruntime.1.dylib"
cp "$built" "$lib"
install_name_tool -id @rpath/libonnxruntime.1.dylib "$lib"
ln -sf libonnxruntime.1.dylib "$out/libonnxruntime.dylib"   # what -l onnxruntime opens
# install_name_tool invalidates the signature; ad-hoc re-sign or the loader refuses it.
codesign --force --sign - "$lib"

archs=$(lipo -archs "$lib")
[[ $archs == x86_64 ]] || { echo "error: lipo -archs = '$archs', want exactly 'x86_64'" >&2; exit 1; }

id=$(otool -D "$lib" | sed -n '2p')
[[ $id == @rpath/libonnxruntime.1.dylib ]] || { echo "error: install name is '$id', want '@rpath/libonnxruntime.1.dylib'" >&2; exit 1; }

# Skip otool -L's header line and the install-name line; every remaining entry is
# a dependency, and an absolute CI path there would break on the user's machine.
deps=$(otool -L "$lib" | tail -n +3 | awk '{print $1}')
while read -r dep; do
  case $dep in
    "") continue ;;
    /usr/lib/* | /System/*) ;;
    *) echo "error: dependency '$dep' is not a system library, and only this dylib is bundled" >&2; exit 1 ;;
  esac
done <<<"$deps"

echo "LC_BUILD_VERSION minos: $(otool -l "$lib" | awk '/LC_BUILD_VERSION/{f=1} f&&/minos/{print $2; exit}')"
echo "built $lib"
file "$lib"
otool -L "$lib"
