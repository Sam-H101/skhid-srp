#!/bin/sh
# dis.sh VERSION VA LEN : see dis.py.
exec python "$(dirname "$0")/dis.py" "$@"
