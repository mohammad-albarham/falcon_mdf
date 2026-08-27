import pathlib
import subprocess
import tempfile

import pytest

import falcon_mdf

REPO_ROOT = pathlib.Path(__file__).parent.parent.parent


@pytest.fixture(scope="session")
def sample_mf4() -> pathlib.Path:
    """Generate a tiny MF4 fixture at runtime using falcon's own writer."""
    with tempfile.TemporaryDirectory() as tmpdir:
        out = pathlib.Path(tmpdir) / "smoke.mf4"
        try:
            subprocess.run(
                [
                    "cargo",
                    "run",
                    "--example",
                    "write_mf4",
                    "--",
                    str(out),
                ],
                cwd=REPO_ROOT,
                check=True,
                capture_output=True,
                text=True,
            )
        except (subprocess.CalledProcessError, FileNotFoundError) as exc:
            pytest.skip(f"could not build runtime MF4 fixture: {exc}")

        if not out.exists():
            pytest.skip("runtime MF4 fixture was not created")

        yield out


def test_smoke(sample_mf4: pathlib.Path) -> None:
    f = falcon_mdf.open(str(sample_mf4))
    info = f.info()
    channels = f.channels()

    assert isinstance(info["version"], str)
    assert info["channel_group_count"] >= 1
    assert info["channel_count"] == len(channels)
    assert info["sample_count"] > 0

    values, timestamps = f.get(channels[0])
    assert len(values) == len(timestamps)
    assert len(values) == info["sample_count"]


def test_to_dataframe(sample_mf4: pathlib.Path) -> None:
    """Round-trip decoded channels through to_dataframe() and compare with get()."""
    pd = pytest.importorskip("pandas")
    pytest.importorskip("pyarrow")

    f = falcon_mdf.open(str(sample_mf4))
    channels = f.channels()

    df = f.to_dataframe()
    assert isinstance(df, pd.DataFrame)
    assert "time" in df.columns
    for name in channels:
        assert name in df.columns

    # Values and timestamps for each channel must match the direct get() path.
    # Invalid samples are exported as Arrow nulls, while get() returns the raw
    # stand-in values, so only non-null positions are compared.
    for name in channels:
        values, _timestamps = f.get(name)
        valid = df[name].notna()
        pd.testing.assert_series_equal(
            df[name][valid],
            pd.Series(values, name=name)[valid.values],
            check_exact=False,
            check_names=False,
        )
    _values, timestamps = f.get(channels[0])
    pd.testing.assert_series_equal(
        df["time"],
        pd.Series(timestamps, name="time"),
        check_exact=False,
        check_names=False,
    )

    # Selected-channel subset works and keeps the same values.
    subset = f.to_dataframe(channels=[channels[0]])
    assert list(subset.columns) == ["time", channels[0]]
    pd.testing.assert_series_equal(
        subset[channels[0]],
        df[channels[0]],
        check_exact=False,
    )


def test_to_dataframe_invalid_samples(sample_mf4: pathlib.Path) -> None:
    """Invalid samples from the fixture must surface as pandas nulls."""
    pd = pytest.importorskip("pandas")
    pytest.importorskip("pyarrow")

    f = falcon_mdf.open(str(sample_mf4))
    if "Boost" not in f.channels():
        pytest.skip("fixture does not contain the invalid-sample Boost channel")

    df = f.to_dataframe()
    assert isinstance(df, pd.DataFrame)
    # The write_mf4 example marks samples 40..60 of Boost as invalid.
    assert df["Boost"].isna().sum() == 20


def test_to_dataframe_polars(sample_mf4: pathlib.Path) -> None:
    """polars backend returns a polars DataFrame with matching shape."""
    pytest.importorskip("polars")
    pytest.importorskip("pyarrow")

    import polars as pl

    f = falcon_mdf.open(str(sample_mf4))
    df = f.to_dataframe(backend="polars")
    assert isinstance(df, pl.DataFrame)
    assert "time" in df.columns
    for name in f.channels():
        assert name in df.columns
    assert df.height == f.info()["sample_count"]


def test_iter_to_dataframe_pandas(sample_mf4: pathlib.Path) -> None:
    """Concatenating yielded DataFrames equals to_dataframe() for various chunk sizes."""
    pd = pytest.importorskip("pandas")
    pytest.importorskip("pyarrow")

    f = falcon_mdf.open(str(sample_mf4))
    expected_df = f.to_dataframe()

    for chunk_size in [1, 7, 13, 25, 33, 50, 100, 150]:
        frames = list(f.iter_to_dataframe(chunk_size=chunk_size))
        assert len(frames) > 0

        # Each frame must be a DataFrame with at most chunk_size rows
        for df in frames:
            assert isinstance(df, pd.DataFrame)
            assert len(df) <= chunk_size

        concatenated = pd.concat(frames, ignore_index=True)
        pd.testing.assert_frame_equal(concatenated, expected_df)


def test_iter_to_dataframe_subset_channels(sample_mf4: pathlib.Path) -> None:
    """iter_to_dataframe with channel selection matches to_dataframe with same selection."""
    pd = pytest.importorskip("pandas")
    pytest.importorskip("pyarrow")

    f = falcon_mdf.open(str(sample_mf4))
    subset = ["Speed"]

    expected_df = f.to_dataframe(channels=subset)
    frames = list(f.iter_to_dataframe(chunk_size=17, channels=subset))
    concatenated = pd.concat(frames, ignore_index=True)
    pd.testing.assert_frame_equal(concatenated, expected_df)


def test_iter_to_dataframe_polars(sample_mf4: pathlib.Path) -> None:
    """Concatenating polars streaming frames equals to_dataframe(backend='polars')."""
    pytest.importorskip("polars")
    pytest.importorskip("pyarrow")

    import polars as pl

    f = falcon_mdf.open(str(sample_mf4))
    expected_df = f.to_dataframe(backend="polars")

    for chunk_size in [1, 11, 25, 50, 100]:
        frames = list(f.iter_to_dataframe(chunk_size=chunk_size, backend="polars"))
        assert len(frames) > 0
        for df in frames:
            assert isinstance(df, pl.DataFrame)
            assert df.height <= chunk_size

        concatenated = pl.concat(frames)
        assert concatenated.equals(expected_df)


def test_iter_to_dataframe_invalid_arguments(sample_mf4: pathlib.Path) -> None:
    """Invalid arguments to iter_to_dataframe raise ValueError."""
    pytest.importorskip("pyarrow")

    f = falcon_mdf.open(str(sample_mf4))

    with pytest.raises(ValueError, match="chunk_size must be greater than 0"):
        f.iter_to_dataframe(chunk_size=0)

    with pytest.raises(ValueError, match="unsupported backend"):
        f.iter_to_dataframe(chunk_size=10, backend="invalid_backend")

