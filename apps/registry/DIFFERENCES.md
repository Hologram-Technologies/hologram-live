# Differences from the reference registry

Hologram Registry is measured against the Docker Registry image, pinned by digest in
[`gates/reference.env`](gates/reference.env). One scripted session is played against both and compared field by
field: status, the headers that carry meaning, and the body. This file lists every difference that is kept on
purpose, and why. A difference that is not listed here fails the gate. So does a line here that no longer happens.

The table is read by the gate. Keep its two markers and its seven columns.

<!-- gate-b:begin -->
| id | scenario | step | field | reference | product | reason |
|---|---|---|---|---|---|---|
<!-- gate-b:end -->
