# Wasm + View `.holo` application

This example composes an exit-bearing Wasm primary with a portable View layer.
The frontend sends `application.invoke` intent v1 messages to the primary; the
primary returns an ASCII-uppercase transformation. The View has no ambient
Tauri, shell, filesystem, network, or localhost authority.

Compile it as one self-contained archive:

```bash
hologram compile examples/wasm-view/hologram.json \
  --output target/wasm-view.holo
```

Import `target/wasm-view.holo` from Hologram Desktop and choose **Open
application**. Its portable frontend opens in a separate native window outside
the dashboard, and repeated submissions reuse the same prepared Wasm primary.
Close that window or choose **Stop application** to detach the View and stop
the session. Direct CLI execution deliberately reports that the portable View
surface is unavailable because a headless process cannot attach the UI.

The runtime and Desktop adapter tests compile this exact manifest, attach its
bundle through a display-independent window host, invoke the primary directly
and through real View intents while the session remains open, then prove
explicit and window-driven reverse shutdown. The CLI's one-shot executor still
uses fresh start/invoke/stop semantics; the explicit session API is what owns a
user-driven View lifetime.
