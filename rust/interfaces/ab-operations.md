# Gateway blob grant parity

Blob reads return Unauthorized when the supplied record is absent, invisible, deleted, or does not reference the requested digest. Authorized records whose bytes are unavailable still return NotFound. This preserves the Go frontdoor's HTTP 401 behavior for invalid record grants without changing record reads or the wire format.
