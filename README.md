# EMF HLR importer

This provisions SIMs from CSV files similar to what we get from c3gsm.
Format should be either comma or space separated and have at least columns "IMSI", "KI" and "OPC".

Saves default MSISDNs to ./defaults.csv, saves keys into the osmo-hlr on localhost.

# How to use me

$ cargo run --release -- my_keys.csv [my_keys_2.csv...]
