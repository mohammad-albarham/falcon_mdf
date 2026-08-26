# falcon_mdf Python bindings

Python bindings for the `falcon_mdf` Rust MF4 reader.

```python
import falcon_mdf

f = falcon_mdf.open("measurement.mf4")
print(f.info())
print(f.channels())
values, timestamps = f.get("VehicleSpeed")
```
