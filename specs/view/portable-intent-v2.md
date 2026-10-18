# Explicit bounded portable View intent version 2

Version 1 retains its decoded UTF-8 payload limit of 65,536 bytes, total UTF-8 output limit of 1,048,576 bytes, exact `application.invoke` name and response version 1.

Version 2 admits a decoded UTF-8 payload of at most 2,097,152 bytes and total UTF-8 outputs of at most 4,194,304 bytes. The request and response shapes remain closed, and the response echoes the accepted request version. Both versions retain the 16-output limit, application attachment identity, serialized primary invocation, UTF-8 validation and lifecycle authority. Unknown versions and names fail before primary invocation. This is a bounded text transport; it grants no file, network, process or persistent-state authority.

The host-neutral `intent_limits` function is the single per-version budget authority. The real application intent broker uses it for total output enforcement after invoking its bound primary. Consumer HTTP adapters must additionally bound their encoded envelope before decoding and validate the decoded request through this API; encoded JSON expansion does not increase the decoded byte budget.

The source-owned application continues to own its input language, resource model, results, case identities and recomputation. Selecting transport version 2 cannot raise an application's own guest allocation or response limits.
