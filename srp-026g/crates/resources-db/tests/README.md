`fixture.db`: five files packed by resource-porter's `resources-db pack`
(`deps/vostok-resources-db`, resource-porter commit 5d3b53e) from this tree:

```
gameplay/first.txt                     "first file\n"
gameplay/items/item.options            "an item\n"
gameplay/items/same_bytes.options      "an item\n"
gameplay/items/big.bin                 5000 times 'x'
projects/level_01/day/client_project   empty
```

`release.db`: the same tree with `gameplay/items/new.options` ("a new
item\n") added, packed the same way: the archive a release makes of
`fixture.db` (its first 9115 bytes as a region, 11 more as the tail).
