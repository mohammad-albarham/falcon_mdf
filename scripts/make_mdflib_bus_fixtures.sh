#!/usr/bin/env bash
# Builds ihedvall/mdflib at a pinned commit and uses it to write the Ethernet
# and FlexRay bus-logging fixtures tests/bus_mdflib_fixtures.rs reads:
#
#   test_data/generated/mdflib_ethernet.mf4
#   test_data/generated/mdflib_flexray.mf4
#
# mdflib is an independent C++ MDF implementation, so these are the only
# Ethernet and FlexRay files in the test set this crate did not write itself.
# Generated rather than committed: measurement files are never checked in.
#
# Needs git, cmake, a C++20 compiler, zlib and expat (on Debian/Ubuntu:
# `apt-get install cmake g++ zlib1g-dev libexpat1-dev`). The mdflib build is
# cached under target/mdflib; delete it to rebuild.
set -euo pipefail

MDFLIB_REPO="https://github.com/ihedvall/mdflib.git"
MDFLIB_COMMIT="73e2f91"

root="$(cd "$(dirname "$0")/.." && pwd)"
cache="$root/target/mdflib"
src="$cache/src"
build="$cache/build"
out="$root/test_data/generated"

if [ ! -d "$src/.git" ]; then
  git clone --quiet "$MDFLIB_REPO" "$src"
fi
git -C "$src" fetch --quiet --depth 1 origin "$MDFLIB_COMMIT" 2>/dev/null || true
git -C "$src" checkout --quiet "$MDFLIB_COMMIT"

# Shared, because mdflib's zlib script asks for a static zlib when building
# static and macOS ships none. -Wno-invalid-constexpr: a constexpr in mdflib's
# idatawriter.h that recent clang rejects; the code is never constant-evaluated.
if [ ! -f "$build/mdflib/libmdf.so" ] && [ ! -f "$build/mdflib/libmdf.dylib" ]; then
  cmake -S "$src" -B "$build" -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=ON \
    -DCMAKE_CXX_FLAGS="-Wno-invalid-constexpr" > "$cache/cmake.log"
  cmake --build "$build" -j "$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)" > "$cache/build.log"
fi

cxx="${CXX:-c++}"
"$cxx" -std=c++20 -O1 -Wno-invalid-constexpr \
  -I"$src/include" -I"$src/mdflib/src" \
  "$root/scripts/mdflib_bus_fixtures/main.cpp" \
  -L"$build/mdflib" -lmdf -Wl,-rpath,"$build/mdflib" \
  -o "$cache/bus_fixtures"

mkdir -p "$out"
"$cache/bus_fixtures" "$out/mdflib_ethernet.mf4" "$out/mdflib_flexray.mf4"
echo "wrote $out/mdflib_ethernet.mf4 and $out/mdflib_flexray.mf4 (mdflib $MDFLIB_COMMIT)"
