---
title: Apps
description: Apps that run entirely in your browser, each in its own sealed frame, with every model byte verified before it is used.
group: Concepts
order: 14
---

An App is an app and the model it needs, sealed under one address and run by the visitor's own browser. Nothing runs on a server: the page is a folder of static files, the model comes from the host the App names, and the work happens on the visitor's GPU. The hub lists three at [/spaces/](/spaces/): speech, image and chat.

## What is sealed

Every file in an App's folder is named in `holospace.lock.json` with its SHA-256; the runtime files it depends on (the verified fetch, the store, its ONNX Runtime build) are named there too. The lock's `root` is the SHA-256 over that map. Change one byte anywhere and the root changes. The catalog at [/spaces/spaces.json](/spaces/spaces.json) carries each root.

## How the model is verified

The App names its model (`onnx-community/Kokoro-82M-v1.0-ONNX`, say). Before the app's code runs, a small prelude reads the model index — `/api/models/{owner}/{name}/tree/main` on the model host — and from then on every fetch of a model file is held until its bytes re-derive to the digest the index gave for that path: SHA-256 for LFS files, git's blob SHA-1 for the small ones. A byte that does not match is refused; the app sees a failed load, never a wrong file. A verified file is kept in the App's own browser store by its digest, so the second open needs no network.

The host is a location, not the identity. Today the demo Apps read from huggingface.co; when the hub indexes the same models, the same Apps read from here with the same digests.

## Isolation

Each App runs in a sandboxed frame on this origin with its own storage namespace, and a Content-Security-Policy written into its page allows exactly three things: its own files, the hub's brand kit, and the one model host it declares. No CDN script, no third-party asset, no analytics. The frame asks for no camera or microphone.

## On the registry

A published App is an OCI artifact at `/v2/spaces/<id>`: config `holospace.json`, one layer per file, artifact type `application/vnd.hologram.space.v1+json`, the sealed root in an annotation, tag `latest`. Reads need no token. The page asks the registry for each App's manifest and marks the ones it finds.

```bash
curl -sI https://gethologram.ai/v2/spaces/kokoro-tts/manifests/latest \
  -H "Accept: application/vnd.oci.image.manifest.v1+json" | grep -i docker-content-digest
```

## What it needs

WebGPU and private file storage (OPFS): current Chrome, Edge or Safari on a device with a GPU. The page says so on the card when a browser lacks either. Model downloads are 176–390 MB on first open and come from the store after that; the first result waits on the GPU compiling the model, which can take a minute on integrated graphics.
