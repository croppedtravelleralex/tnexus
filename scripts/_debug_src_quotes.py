from pathlib import Path

t = Path(r"D:\SelfMadeTool\TNexus\crates\grok-signer\assets\grok_sign_standalone.js").read_text(encoding="utf-8")
i = t.index("const src = '")
j = t.index("';", i + 12)
s = t[i + 12 : j]
print("len", len(s))
# find first problematic char for JS single-quoted string
for k, ch in enumerate(s):
    if ch == "'" and (k == 0 or s[k - 1] != "\\"):
        print("bare quote at", k, repr(s[max(0, k - 20) : k + 20]))
        break
else:
    print("no bare single quotes")
