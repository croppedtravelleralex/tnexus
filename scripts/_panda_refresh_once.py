#!/usr/bin/env python3
import sys
sys.path.insert(0, "/app")
from services.account_service import account_service
from services.account_refresh_all_service import account_refresh_all_service

all_tokens = account_service.list_tokens()
eligible = []
for token in all_tokens:
    account = account_service.get_account(token) or {}
    if str(account.get("last_token_refresh_error") or "").strip():
        continue
    eligible.append(token)
print(f"eligible={len(eligible)}/{len(all_tokens)}", flush=True)
result = account_service.refresh_accounts(eligible, None, False, False)
errs = result.get("errors") or []
print(f"refreshed={result.get('refreshed', 0)} errors={len(errs)}", flush=True)
for e in errs[:8]:
    print(" err:", e)
sync = account_refresh_all_service.sync_last_refreshed_accounts_to_panda()
print(f"panda_sync={sync}", flush=True)
