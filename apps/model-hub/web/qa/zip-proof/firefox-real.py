#!/usr/bin/env python3
"""Transport proof in stock Firefox (selenium/standalone-firefox), driven over plain WebDriver HTTP.

Playwright's Firefox build keeps any large download in memory (an ordinary 6 GB server download exhausts it too), so
it cannot judge memory. This runs the released browser instead.

  docker run -d --name zp-ff --memory 2g --cpus 2 --shm-size 1g -v /root/hub/tm/zp:/proof selenium/standalone-firefox
  docker exec -d zp-ff python3 -m http.server 8099 --directory /proof/site
  python3 firefox-real.py <container ip> <link id, e.g. go-6gnet>
"""
import json, os, subprocess, sys, time, urllib.request

host, link = sys.argv[1], sys.argv[2]
base = f"http://{host}:4444"


def call(method, path, body=None):
    request = urllib.request.Request(base + path, method=method, data=json.dumps(body).encode() if body is not None else None, headers={"content-type": "application/json"})
    return json.load(urllib.request.urlopen(request, timeout=120))["value"]


prefs = {"browser.download.folderList": 2, "browser.download.dir": "/proof/out", "browser.download.useDownloadDir": True,
         "browser.helperApps.neverAsk.saveToDisk": "application/zip,application/octet-stream", "browser.download.manager.showWhenStarting": False,
         "browser.download.always_ask_before_handling_new_types": False}
session = call("POST", "/session", {"capabilities": {"alwaysMatch": {"browserName": "firefox", "moz:firefoxOptions": {"args": ["-headless"], "prefs": prefs}}}})
sid = session["sessionId"]
result = {"engine": "firefox (stock)", "version": session["capabilities"].get("browserVersion"), "link": link}
try:
    call("POST", f"/session/{sid}/url", {"url": "http://localhost:8099/"})
    for _ in range(60):
        if call("POST", f"/session/{sid}/execute/sync", {"script": "return window.ready === true", "args": []}):
            break
        time.sleep(1)
    declared = call("POST", f"/session/{sid}/execute/sync", {"script": "return window.expected['6g']", "args": []})
    result["declared"] = declared
    started = time.time()
    call("POST", f"/session/{sid}/execute/sync", {"script": f"document.getElementById('{link}').click()", "args": []})
    peak, size, target = 0, 0, "/root/hub/tm/zp/out/proof-6g.zip"
    while time.time() - started < 1500:
        time.sleep(3)
        stats = subprocess.run(["docker", "stats", "--no-stream", "--format", "{{.MemUsage}}", "zp-ff"], capture_output=True, text=True).stdout.split("/")[0].strip()
        mib = float(stats[:-3]) * (1024 if stats.endswith("GiB") else 1) if stats else 0
        peak = max(peak, mib)
        size = os.path.getsize(target) if os.path.exists(target) else 0
        part = target + ".part"
        if size == declared and not os.path.exists(part):
            break
        if subprocess.run(["docker", "inspect", "-f", "{{.State.Running}}", "zp-ff"], capture_output=True, text=True).stdout.strip() != "true":
            result["error"] = "the browser container died"
            break
    result.update(seconds=round(time.time() - started), saved=size, sizeMatches=size == declared, peakContainerMB=round(peak))
    result["progress"] = call("POST", f"/session/{sid}/execute/sync", {"script": "return window.progress", "args": []})
except Exception as error:  # noqa: BLE001
    result["error"] = str(error)[:200]
finally:
    try:
        call("DELETE", f"/session/{sid}")
    except Exception:  # noqa: BLE001
        pass
print(json.dumps(result))
