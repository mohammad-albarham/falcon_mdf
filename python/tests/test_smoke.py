import pathlib

import falcon_mdf

DATA_DIR = pathlib.Path(__file__).parent.parent.parent / "test_data"
MF4_PATH = DATA_DIR / "smoke.mf4"


def test_smoke():
    f = falcon_mdf.open(str(MF4_PATH))
    info = f.info()
    channels = f.channels()

    assert isinstance(info["version"], str)
    assert info["channel_group_count"] >= 1
    assert info["channel_count"] == len(channels)
    assert info["sample_count"] > 0

    values, timestamps = f.get(channels[0])
    assert len(values) == len(timestamps)
    assert len(values) == info["sample_count"]
