"""Checks srp's stored-chunk LZMA2 streams (crates/survarium/src/v026g/lzma2.rs)
against a real decoder: liblzma's raw LZMA2 filter with the 0.26g client's
dictionary size (32 KiB, property byte 6).

    python -I check_lzma2.py <dir>

<dir> holds pairs `<name>.bin` (the data) and `<name>.lzma2` (the stream);
the test `a_real_decoder_reads_the_streams` writes them and runs this.
"""
import glob
import lzma
import os
import sys

FILTERS = [{"id": lzma.FILTER_LZMA2, "dict_size": 0x8000}]

count = 0
for stream_path in sorted(glob.glob(os.path.join(sys.argv[1], "*.lzma2"))):
    with open(stream_path, "rb") as file:
        stream = file.read()
    with open(stream_path[: -len(".lzma2")] + ".bin", "rb") as file:
        data = file.read()
    decoder = lzma.LZMADecompressor(format=lzma.FORMAT_RAW, filters=FILTERS)
    decoded = decoder.decompress(stream)
    name = os.path.basename(stream_path)
    if decoded != data:
        sys.exit("%s: decoded %d bytes, expected %d" % (name, len(decoded), len(data)))
    if not decoder.eof or decoder.unused_data:
        sys.exit("%s: the stream does not end at its end marker" % name)
    count += 1
print("%d streams decoded" % count)
