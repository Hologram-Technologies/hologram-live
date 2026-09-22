Throwaway TLS material for tests and gates only. The keys are public on purpose.

- ca.crt / ca.key: a test CA.
- server.crt / server.key: signed by it, for localhost, 127.0.0.1 and registry.gate, valid 100 years.
- chain.crt: server.crt followed by ca.crt (the listener sends a whole chain).
