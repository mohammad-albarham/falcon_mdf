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
