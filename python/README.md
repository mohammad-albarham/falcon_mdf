# falcon_mdf Python bindings

Python bindings for the `falcon_mdf` Rust MF4 reader.

```python
import falcon_mdf

f = falcon_mdf.open("measurement.mf4")
print(f.info())
print(f.channels())
values, timestamps = f.get("VehicleSpeed")
```

## DataFrames

Decoded channels can be handed directly to pandas or polars as an Arrow IPC
table:

```python
import falcon_mdf

f = falcon_mdf.open("measurement.mf4")

# pandas DataFrame (default)
df = f.to_dataframe()

# Only selected channels
df = f.to_dataframe(channels=["VehicleSpeed", "EngineSpeed"])

# polars DataFrame
df = f.to_dataframe(backend="polars")
```

`to_dataframe()` requires `pyarrow` and the chosen backend (`pandas` or
`polars`) to be installed. Channels that do not already share one time axis are
resampled onto the first requested channel's time axis with linear
interpolation.
