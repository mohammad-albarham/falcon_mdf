# Python

The Python binding uses the Rust reader and provides direct channel access and DataFrame conversion. It is developed in the repository's `python/` package; build it locally with [maturin](https://www.maturin.rs/):

```sh
python3 -m venv .venv
.venv/bin/python -m pip install maturin
.venv/bin/python -m maturin develop --manifest-path python/Cargo.toml
```

The first command creates a local environment. Run the remaining commands from the repository root. The binding is separate from the Rust crate's installation.

## Read a channel

```python
import falcon_mdf

measurement = falcon_mdf.open("measurement.mf4")
print(measurement.info())
print(measurement.channels())
values, timestamps = measurement.get("VehicleSpeed")
```

## Convert to a DataFrame

Install `pyarrow` and either `pandas` or `polars` in the same environment:

```python
frame = measurement.to_dataframe(channels=["VehicleSpeed", "EngineSpeed"])
polars_frame = measurement.to_dataframe(backend="polars")
```

Channels that do not share a time axis are resampled onto the first requested channel's axis with linear interpolation. For files too large to materialize at once, use `iter_to_dataframe(chunk_size=50_000, channels=[...])`. Streaming selections must belong to the same channel group.

See the [Python binding README](https://github.com/mohammad-albarham/falcon_mdf/blob/main/python/README.md) for more examples.
