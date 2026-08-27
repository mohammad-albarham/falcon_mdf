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

### Streaming DataFrames

For files too large to hold in memory, `iter_to_dataframe()` yields successive
DataFrames in aligned windows of `chunk_size` samples:

```python
# Stream pandas DataFrames in windows of 50,000 samples
for df in f.iter_to_dataframe(chunk_size=50_000):
    process(df)

# Stream selected channels to polars DataFrames
for df in f.iter_to_dataframe(
    chunk_size=100_000,
    channels=["VehicleSpeed", "EngineSpeed"],
    backend="polars",
):
    process(df)
```

Channels passed to `iter_to_dataframe()` must belong to the same channel group so
that chunks are aligned by sample index without materializing the whole file.

