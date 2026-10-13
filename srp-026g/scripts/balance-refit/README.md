# Balance refit

Fits the team-balance weights (`docs/notes/matchmaking.md` section 8) to what
the server logged: the "sides" line of each match (what every player counted
for) and its "settled" line (the winner). Python 3 only, no packages; prints no
player names.

```
# copy the day logs off the host (they are in the srp data volume, logs/srp-YYYY-MM-DD.log)
python scripts/balance-refit/refit.py LOGDIR --since 2026-10-13T21:31
python scripts/balance-refit/refit.py LOGDIR --weights 10,1,2,4 --weights 8,1,2.5,4 --strict
```

- Free fit: the weights that would have predicted the winners best, in the
  server's units (gear = 1) with a bootstrap range.
- Scored weights: the ones in force, the ones before, any `--weights P,G,L,R`,
  and each part alone, by log loss on the same matches.
- Use it after about 60 matches on one setting. Under about 40 the free fit
  is noise and the script says so. `--strict` keeps only matches whose
  roster did not change (late joins and leavers make the logged sides not what
  played).
- The logged rating is the one at match start; the server updates it after.
- Needs the numbers in force as `IN_FORCE` at the top of the script: edit it
  when `SRP_BALANCE_*` changes.
- The script checks each match's logged per-player values against the formula
  and prints a NOTE when a match was made with other weights or another trust
  constant (the fit still uses the logged parts).
- Tests: `python -I scripts/balance-refit/test_refit.py` (synthetic logs; checks
  parsing, the since filter, the fit recovering known weights).
