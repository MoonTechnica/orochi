#!/usr/bin/env python3
"""Statusline hook: store only documented quota fields, never transcript paths or identity."""
import json
from pathlib import Path
import sys
import time
value = json.load(sys.stdin)
limits = {}
for key in ('five_hour', 'seven_day', 'spend_limit'):
    source = value.get('rate_limits', {}).get(key)
    if isinstance(source, dict):
        limits[key] = {k: source[k] for k in ('used_percentage', 'resets_at') if k in source}
p = Path(sys.argv[1])
p.write_text(json.dumps({'observed_at': int(time.time()), 'rate_limits': limits}) + '\n')
print('Orochi quota capture')
