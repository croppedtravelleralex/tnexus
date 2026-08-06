"""Default HTTP headers for test scripts calling Cloudflare-protected APIs.

Python urllib sends ``Python-urllib/3.x`` by default, which Cloudflare blocks
with error 1010. Use :func:`request_headers` for all outbound test requests.
"""

from __future__ import annotations


def request_headers(extra: dict[str, str] | None = None) -> dict[str, str]:
    headers = {
        "User-Agent": "curl/8.5.0",
        "Accept": "application/json, text/plain, */*",
    }
    if extra:
        headers.update(extra)
    return headers
