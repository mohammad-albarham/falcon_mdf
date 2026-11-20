"""Regression and feature tests for PY-01 through PY-04."""

from __future__ import annotations

import gc
import math
import os
import pathlib
import threading

import pytest

import falcon_mdf


class _CustomPathLike(os.PathLike[str]):
    """Minimal os.PathLike wrapper to verify os.fspath handling."""

    def __init__(self, inner: pathlib.Path) -> None:
        self._inner = inner

    def __fspath__(self) -> str:
        return str(self._inner)


# ---------------------------------------------------------------------------
# Fixtures (reuse write_mf4 from test_smoke or build synthetic files via asammdf)
# ---------------------------------------------------------------------------


@pytest.fixture(scope="module")
def rich_mf4(tmp_path_factory: pytest.TempPathFactory) -> pathlib.Path:
    """MF4 with i64/u64 > 2^53, UTF-8 text, fixed bytes, 1D/2D arrays, and invalidation bits."""
    np = pytest.importorskip("numpy")
    asammdf = pytest.importorskip("asammdf")
    MDF, Signal = asammdf.MDF, asammdf.Signal

    out = tmp_path_factory.mktemp("rich") / "rich_types.mf4"
    t = np.array([0.0, 0.1, 0.2, 0.3], dtype=np.float64)
    inv = np.array([False, True, False, True])

    i64_vals = np.array([(1 << 62) + 1234567, 999, -(1 << 62) - 9876543, 888], dtype=np.int64)
    u64_vals = np.array([(1 << 63) + 42, 777, (1 << 64) - 1, 666], dtype=np.uint64)
    f64_vals = np.array([1.5, 9999.0, -2.25, 8888.0], dtype=np.float64)
    str_vals = np.array([b"alpha", b"discarded", b"gamma", b"ignored"])
    bytes_vals = np.array(
        [[0x01, 0x02, 0x03], [0xFF, 0xFF, 0xFF], [0x04, 0x05, 0x06], [0xEE, 0xEE, 0xEE]],
        dtype=np.uint8,
    )
    arr1d_vals = np.rec.fromarrays(
        [np.arange(16, dtype=np.float64).reshape(4, 4)],
        names=["sig_arr1d"],
    )
    arr2d_vals = np.rec.fromarrays(
        [np.arange(24, dtype=np.float64).reshape(4, 2, 3)],
        names=["sig_arr2d"],
    )

    m = MDF(version="4.10")
    m.append(
        [
            Signal(i64_vals, t, name="sig_i64", invalidation_bits=inv),
            Signal(u64_vals, t, name="sig_u64", invalidation_bits=inv),
            Signal(f64_vals, t, name="sig_f64", invalidation_bits=inv),
            Signal(str_vals, t, name="sig_str", encoding="utf-8", invalidation_bits=inv),
            Signal(bytes_vals, t, name="sig_bytes", invalidation_bits=inv),
            Signal(arr1d_vals, t, name="sig_arr1d", invalidation_bits=inv),
            Signal(arr2d_vals, t, name="sig_arr2d", invalidation_bits=inv),
        ]
    )
    m.save(out, overwrite=True)
    m.close()
    return out


@pytest.fixture(scope="module")
def multigroup_mf4(tmp_path_factory: pytest.TempPathFactory) -> pathlib.Path:
    """Multi-group MF4 with duplicate channel names ('time', 'dup') and a 0-sample group."""
    np = pytest.importorskip("numpy")
    asammdf = pytest.importorskip("asammdf")
    MDF, Signal = asammdf.MDF, asammdf.Signal

    out = tmp_path_factory.mktemp("multi") / "multigroup.mf4"
    m = MDF(version="4.10")

    # Group 0: 0 samples (tests 0-sample group coming first!)
    m.append([Signal(np.array([], dtype=np.float64), np.array([], dtype=np.float64), name="empty_sig")])

    # Group 1: 15 samples at 10 Hz, includes duplicate name 'dup'
    t1 = np.arange(15, dtype=np.float64) * 0.1
    m.append(
        [
            Signal(t1 * 2.0, t1, name="g1_sig"),
            Signal(t1 + 1.0, t1, name="dup"),
        ]
    )

    # Group 2: 25 samples at 20 Hz, includes duplicate name 'dup'
    t2 = np.arange(25, dtype=np.float64) * 0.05
    m.append(
        [
            Signal(t2 * 3.0, t2, name="g2_sig"),
            Signal(t2 + 10.0, t2, name="dup"),
        ]
    )

    m.save(out, overwrite=True)
    m.close()
    return out


@pytest.fixture(scope="module")
def mdf3_file(tmp_path_factory: pytest.TempPathFactory) -> pathlib.Path:
    """MDF 3.30 file for testing v3 reading through falcon_mdf.open()."""
    np = pytest.importorskip("numpy")
    asammdf = pytest.importorskip("asammdf")
    MDF, Signal = asammdf.MDF, asammdf.Signal

    out = tmp_path_factory.mktemp("mdf3") / "sample_v330.mdf"
    t = np.arange(20, dtype=np.float64) * 0.05
    m = MDF(version="3.30")
    m.append(
        [
            Signal(np.arange(20, dtype=np.float64) * 1.5, t, name="v3_speed"),
            Signal(np.arange(20, dtype=np.int32) - 5, t, name="v3_counter"),
        ]
    )
    m.save(out, overwrite=True)
    m.close()
    return out


# ---------------------------------------------------------------------------
# PY-01: Safe DataFrameIterator drop order & frozen Mf4File
# ---------------------------------------------------------------------------


def test_iterator_survives_parent_drop_and_gc(tmp_path: pathlib.Path, rich_mf4: pathlib.Path) -> None:
    """Dropping Mf4File while DataFrameIterator is alive must not UAF or crash."""
    pd = pytest.importorskip("pandas")
    pytest.importorskip("pyarrow")

    # Baseline frame before dropping parent
    with falcon_mdf.open(rich_mf4) as ref:
        expected = ref.to_dataframe(channels=["sig_f64"])

    f = falcon_mdf.open(rich_mf4)
    it = f.iter_to_dataframe(chunk_size=2, channels=["sig_f64"])
    del f
    gc.collect()

    frames = list(it)
    assert len(frames) == 2
    concatenated = pd.concat(frames, ignore_index=True)
    pd.testing.assert_frame_equal(concatenated, expected)

    # Also test dropping an active, partially consumed iterator after parent is gone.
    f2 = falcon_mdf.open(rich_mf4)
    it2 = f2.iter_to_dataframe(chunk_size=1, channels=["sig_f64"])
    del f2
    gc.collect()
    first = next(it2)
    assert len(first) == 1
    del it2
    gc.collect()


def test_iterator_survives_parent_close(rich_mf4: pathlib.Path) -> None:
    """Closing Mf4File keeps an existing DataFrameIterator valid and safe."""
    pd = pytest.importorskip("pandas")
    pytest.importorskip("pyarrow")

    f = falcon_mdf.open(rich_mf4)
    it = f.iter_to_dataframe(chunk_size=2, channels=["sig_f64"])
    f.close()
    assert f.closed is True

    with pytest.raises(ValueError, match="closed"):
        f.get("sig_f64")

    frames = list(it)
    assert sum(len(df) for df in frames) == 4
    assert all(isinstance(df, pd.DataFrame) for df in frames)


def test_concurrent_reads_on_frozen_file(multigroup_mf4: pathlib.Path) -> None:
    """Multiple threads can read the same Mf4File concurrently (frozen + GIL release)."""
    pytest.importorskip("pandas")
    pytest.importorskip("pyarrow")

    errors: list[BaseException] = []

    with falcon_mdf.open(multigroup_mf4) as f:
        def worker() -> None:
            try:
                for _ in range(10):
                    vals, ts = f.get("g1_sig")
                    assert len(vals) == len(ts) == 15
                    df = f.to_dataframe(channels=["g1_sig"])
                    assert len(df) == 15
                    chunks = list(f.iter_to_dataframe(chunk_size=5, group=1))
                    assert sum(len(c) for c in chunks) == 15
            except BaseException as exc:  # pragma: no cover
                errors.append(exc)

        threads = [threading.Thread(target=worker) for _ in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

    assert errors == []


# ---------------------------------------------------------------------------
# PY-02: get() invalidation masking, array nesting, and faithful native types
# ---------------------------------------------------------------------------


def test_get_masks_invalidation_and_preserves_native_types(rich_mf4: pathlib.Path) -> None:
    """get() masks invalid samples (NaN for float, None for non-float) and keeps precision."""
    with falcon_mdf.open(rich_mf4) as f:
        # i64 precision beyond 2^53 + None for invalid samples
        vals_i64, ts = f.get("sig_i64")
        assert len(vals_i64) == len(ts) == 4
        assert type(vals_i64[0]) is int
        assert vals_i64[0] == (1 << 62) + 1234567
        assert vals_i64[1] is None
        assert type(vals_i64[2]) is int
        assert vals_i64[2] == -(1 << 62) - 9876543
        assert vals_i64[3] is None

        # u64 precision beyond 2^53 (up to u64::MAX) + None for invalid samples
        vals_u64, _ = f.get("sig_u64")
        assert type(vals_u64[0]) is int
        assert vals_u64[0] == (1 << 63) + 42
        assert vals_u64[1] is None
        assert type(vals_u64[2]) is int
        assert vals_u64[2] == (1 << 64) - 1
        assert vals_u64[3] is None

        # f64 invalid samples become NaN (not raw stand-in 9999.0 / 8888.0)
        vals_f64, _ = f.get("sig_f64")
        assert vals_f64[0] == pytest.approx(1.5)
        assert math.isnan(vals_f64[1])
        assert vals_f64[2] == pytest.approx(-2.25)
        assert math.isnan(vals_f64[3])

        # UTF-8 text preserved as str, invalid samples as None
        vals_str, _ = f.get("sig_str")
        assert vals_str == ["alpha", None, "gamma", None]

        # Byte array preserved as bytes, invalid samples as None
        vals_bytes, _ = f.get("sig_bytes")
        assert vals_bytes == [b"\x01\x02\x03", None, b"\x04\x05\x06", None]


def test_get_preserves_array_nesting_and_masks_invalid_samples(rich_mf4: pathlib.Path) -> None:
    """1D and 2D array samples keep len(values) == len(timestamps) and per-sample shape."""
    with falcon_mdf.open(rich_mf4) as f:
        # 1D array: shape (4, 4)
        vals_1d, ts_1d = f.get("sig_arr1d")
        assert len(vals_1d) == len(ts_1d) == 4
        assert vals_1d[0] == [0.0, 1.0, 2.0, 3.0]
        assert len(vals_1d[1]) == 4 and all(math.isnan(x) for x in vals_1d[1])
        assert vals_1d[2] == [8.0, 9.0, 10.0, 11.0]
        assert len(vals_1d[3]) == 4 and all(math.isnan(x) for x in vals_1d[3])

        # 2D array: shape (4, 2, 3)
        vals_2d, ts_2d = f.get("sig_arr2d")
        assert len(vals_2d) == len(ts_2d) == 4
        assert vals_2d[0] == [[0.0, 1.0, 2.0], [3.0, 4.0, 5.0]]
        assert len(vals_2d[1]) == 2
        assert all(len(row) == 3 and all(math.isnan(x) for x in row) for row in vals_2d[1])
        assert vals_2d[2] == [[12.0, 13.0, 14.0], [15.0, 16.0, 17.0]]
        assert all(len(row) == 3 and all(math.isnan(x) for x in row) for row in vals_2d[3])


# ---------------------------------------------------------------------------
# PY-03: PathLike, channels=[], multi-group streaming, duplicates, 0-sample,
#        context manager / close(), idiomatic exceptions
# ---------------------------------------------------------------------------


def test_open_accepts_pathlib_and_custom_pathlike(rich_mf4: pathlib.Path) -> None:
    """open() accepts str, pathlib.Path, and arbitrary os.PathLike objects."""
    with falcon_mdf.open(rich_mf4) as f_path:
        ch_path = f_path.channels()
    with falcon_mdf.open(_CustomPathLike(rich_mf4)) as f_like:
        ch_like = f_like.channels()
    with falcon_mdf.open(str(rich_mf4)) as f_str:
        ch_str = f_str.channels()
    assert ch_path == ch_like == ch_str


def test_empty_channels_returns_empty_dataframe_and_iterator(rich_mf4: pathlib.Path) -> None:
    """channels=[] returns an empty DataFrame / empty iterator, never all channels."""
    pd = pytest.importorskip("pandas")
    pl = pytest.importorskip("polars")
    pytest.importorskip("pyarrow")

    with falcon_mdf.open(rich_mf4) as f:
        df_pd = f.to_dataframe(channels=[], backend="pandas")
        assert isinstance(df_pd, pd.DataFrame)
        assert len(df_pd) == 0
        assert list(df_pd.columns) == ["time"]

        df_pl = f.to_dataframe(channels=[], backend="polars")
        assert isinstance(df_pl, pl.DataFrame)
        assert df_pl.height == 0
        assert df_pl.columns == ["time"]

        assert list(f.iter_to_dataframe(chunk_size=10, channels=[], backend="pandas")) == []
        assert list(f.iter_to_dataframe(chunk_size=10, channels=[], backend="polars")) == []


def test_multigroup_to_dataframe_duplicates_and_zero_sample(multigroup_mf4: pathlib.Path) -> None:
    """0-sample groups and duplicate column names work cleanly in both pandas and polars."""
    pd = pytest.importorskip("pandas")
    pl = pytest.importorskip("polars")
    pytest.importorskip("pyarrow")

    with falcon_mdf.open(multigroup_mf4) as f:
        df_pd = f.to_dataframe(backend="pandas")
        df_pl = f.to_dataframe(backend="polars")

        assert isinstance(df_pd, pd.DataFrame)
        assert isinstance(df_pl, pl.DataFrame)
        assert len(df_pd) == df_pl.height == 15
        assert len(set(df_pd.columns)) == len(df_pd.columns)
        assert list(df_pd.columns) == df_pl.columns
        assert "empty_sig" in df_pd.columns
        assert df_pd["empty_sig"].isna().all()
        assert df_pl["empty_sig"].null_count() == 15
        assert "dup" in df_pl.columns
        assert "dup_1" in df_pl.columns


def test_iter_to_dataframe_multigroup_and_group_param(multigroup_mf4: pathlib.Path) -> None:
    """iter_to_dataframe(chunk_size) streams group-by-group by default and supports group=N."""
    pd = pytest.importorskip("pandas")
    pl = pytest.importorskip("polars")
    pytest.importorskip("pyarrow")

    with falcon_mdf.open(multigroup_mf4) as f:
        # Default channels=None streams every non-empty group without crashing
        pd_chunks = list(f.iter_to_dataframe(chunk_size=7, backend="pandas"))
        pl_chunks = list(f.iter_to_dataframe(chunk_size=7, backend="polars"))
        assert sum(len(df) for df in pd_chunks) == 15 + 25
        assert sum(df.height for df in pl_chunks) == 15 + 25
        assert all(isinstance(df, pd.DataFrame) and len(df) <= 7 for df in pd_chunks)
        assert all(isinstance(df, pl.DataFrame) and df.height <= 7 for df in pl_chunks)

        # Explicit group=0 (0-sample group) yields nothing
        assert list(f.iter_to_dataframe(chunk_size=7, group=0)) == []

        # Explicit group=1 streams only group 1 (15 rows)
        g1_chunks = list(f.iter_to_dataframe(chunk_size=6, group=1, backend="polars"))
        assert sum(df.height for df in g1_chunks) == 15
        assert all("g1_sig" in df.columns and "g2_sig" not in df.columns for df in g1_chunks)

        # Explicit group=2 streams only group 2 (25 rows)
        g2_chunks = list(f.iter_to_dataframe(chunk_size=10, group=2, backend="pandas"))
        assert sum(len(df) for df in g2_chunks) == 25
        assert all("g2_sig" in df.columns and "g1_sig" not in df.columns for df in g2_chunks)

        # Out-of-range group raises IndexError
        with pytest.raises(IndexError):
            f.iter_to_dataframe(chunk_size=5, group=99)

        # Explicitly asking for channels from two different groups raises NotImplementedError
        with pytest.raises(NotImplementedError):
            f.iter_to_dataframe(chunk_size=5, channels=["g1_sig", "g2_sig"])


def test_close_and_context_manager(rich_mf4: pathlib.Path) -> None:
    """Mf4File supports close(), closed property, and context manager protocol."""
    f = falcon_mdf.open(rich_mf4)
    assert f.closed is False
    with f as entered:
        assert entered is f
        assert f.closed is False
        assert len(f.channels()) > 0
    assert f.closed is True

    # Calling close() again is idempotent
    f.close()
    assert f.closed is True

    # Operations on a closed file raise ValueError
    with pytest.raises(ValueError, match="closed"):
        f.channels()
    with pytest.raises(ValueError, match="closed"):
        f.info()
    with pytest.raises(ValueError, match="closed"):
        f.get("sig_f64")
    with pytest.raises(ValueError, match="closed"):
        f.to_dataframe()
    with pytest.raises(ValueError, match="closed"):
        f.iter_to_dataframe(chunk_size=10)
    with pytest.raises(ValueError, match="closed"):
        with f:
            pass

    # Exceptions inside a `with` block propagate and still close the file
    f2 = falcon_mdf.open(rich_mf4)
    with pytest.raises(ZeroDivisionError):
        with f2:
            _ = 1 / 0
    assert f2.closed is True


def test_exception_mapping(tmp_path: pathlib.Path, rich_mf4: pathlib.Path) -> None:
    """Mf4Error variants map to idiomatic Python exceptions."""
    # Missing file -> FileNotFoundError
    with pytest.raises(FileNotFoundError):
        falcon_mdf.open(tmp_path / "does_not_exist.mf4")

    # Corrupt/non-MDF file -> ValueError
    bogus = tmp_path / "not_an_mf4.mf4"
    bogus.write_bytes(b"definitely not an MDF file" * 8)
    with pytest.raises(ValueError):
        falcon_mdf.open(bogus)

    # Unknown channel -> KeyError
    with falcon_mdf.open(rich_mf4) as f:
        with pytest.raises(KeyError):
            f.get("no_such_channel")
        with pytest.raises(KeyError):
            f.to_dataframe(channels=["no_such_channel"])
        with pytest.raises(KeyError):
            f.iter_to_dataframe(chunk_size=10, channels=["no_such_channel"])


# ---------------------------------------------------------------------------
# PY-04: MDF 3.x and Zstd / LZ4 MF4 support
# ---------------------------------------------------------------------------


def test_open_mdf3_file(mdf3_file: pathlib.Path) -> None:
    """Python binding opens MDF 3.30 files via open(), get(), to_dataframe(), iter_to_dataframe()."""
    pd = pytest.importorskip("pandas")
    pl = pytest.importorskip("polars")
    pytest.importorskip("pyarrow")

    with falcon_mdf.open(mdf3_file) as f:
        info = f.info()
        assert info["version"].startswith("3.")
        assert info["channel_group_count"] == 1
        assert info["sample_count"] == 20

        vals, ts = f.get("v3_speed")
        assert len(vals) == len(ts) == 20
        assert vals[0] == pytest.approx(0.0)
        assert vals[19] == pytest.approx(19 * 1.5)

        cnt, _ = f.get("v3_counter")
        assert cnt == [i - 5 for i in range(20)]

        df_pd = f.to_dataframe(backend="pandas")
        df_pl = f.to_dataframe(backend="polars")
        assert isinstance(df_pd, pd.DataFrame) and len(df_pd) == 20
        assert isinstance(df_pl, pl.DataFrame) and df_pl.height == 20

        chunks_pd = list(f.iter_to_dataframe(chunk_size=6, backend="pandas"))
        chunks_pl = list(f.iter_to_dataframe(chunk_size=6, backend="polars"))
        assert sum(len(c) for c in chunks_pd) == 20
        assert sum(c.height for c in chunks_pl) == 20


@pytest.mark.parametrize("algo_name", ["ZSTD", "TRANSPOSED_ZSTD", "LZ4", "TRANSPOSED_LZ4"])
def test_open_zstd_and_lz4_compressed_mf4(tmp_path: pathlib.Path, algo_name: str) -> None:
    """Python binding decodes Zstd- and LZ4-compressed MF4 4.20 files."""
    np = pytest.importorskip("numpy")
    asammdf = pytest.importorskip("asammdf")
    from asammdf.blocks.v4_constants import CompressionAlgorithm

    out = tmp_path / f"compressed_{algo_name.lower()}.mf4"
    t = np.arange(128, dtype=np.float64) * 0.01
    expected = np.sin(t) * 100.0

    m = asammdf.MDF(version="4.20")
    m.append([asammdf.Signal(expected, t, name="wave")])
    m.save(out, compression=getattr(CompressionAlgorithm, algo_name), overwrite=True)
    m.close()

    with falcon_mdf.open(out) as f:
        vals, ts = f.get("wave")
        assert len(vals) == len(ts) == 128
        assert vals == pytest.approx(expected.tolist())
