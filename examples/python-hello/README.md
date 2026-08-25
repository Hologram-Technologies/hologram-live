# Python hello `.holo` example

This is the smallest Python source project that Hologram can compile into a
rootfs layer. Its `python_hello_holo:main` entrypoint accepts UTF-8 bytes and
returns a JSON byte string.

Compilation and execution currently require a running Docker-compatible engine:

```bash
hologram compile --check hologram.json
hologram run . --input-text "Ada" --output-format json
```

To build an immutable artifact first:

```bash
hologram compile hologram.json --output python-hello.holo
hologram run python-hello.holo --input-text "Ada" --output-format json
```

Expected output:

```json
{
  "message": "Hello, Ada!",
  "name": "Ada",
  "runtime": "python"
}
```
