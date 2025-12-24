from asammdf import MDF

mdf = MDF('test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4')
data = mdf.get('CAN_DataFrame')

print(data)
