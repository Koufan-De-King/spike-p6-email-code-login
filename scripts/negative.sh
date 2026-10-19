#!/usr/bin/env bash
# Negative paths against a running stack: every line must print PASS.
set -uo pipefail
ISS=${ISSUER:-http://127.0.0.1:8080}
MP=${MAILPIT_URL:-http://127.0.0.1:8025}
CH=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM   # S256 of the RFC 7636 example verifier
VER=dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk
fail=0
check() { if [ "$2" = "$3" ]; then echo "PASS $1"; else echo "FAIL $1 (got $2, want $3)"; fail=1; fi; }
auth() { curl -s -o /dev/null -w '%{http_code}' "$ISS/authorize?response_type=code&client_id=another-app&scope=openid&code_challenge=$CH&code_challenge_method=S256&$1"; }

check "localhost redirect refused (RFC 8252 §8.3)"      "$(auth 'redirect_uri=http://localhost:5555/callback')" 400
check "127.0.0.2 refused"                                "$(auth 'redirect_uri=http://127.0.0.2:5555/callback')" 400
check "loopback on another path refused (rule 5)"        "$(auth 'redirect_uri=http://127.0.0.1:5555/other')" 400
check "unregistered custom scheme refused"               "$(auth 'redirect_uri=evil.app:/cb')" 400
check "any loopback port admitted"                       "$(auth 'redirect_uri=http://127.0.0.1:5555/callback')" 303
check "[::1] refused unless registered (rule 5 compares host)" "$(auth 'redirect_uri=http://%5B::1%5D:5555/callback')" 400
check "registered private-use scheme admitted"           "$(auth 'redirect_uri=digital.camer.another:/oauth2redirect')" 303

loc=$(curl -s -o /dev/null -w '%{redirect_url}' "$ISS/authorize?response_type=code&client_id=another-app&scope=openid&redirect_uri=http://127.0.0.1:5555/callback&code_challenge_method=plain&code_challenge=$CH")
check "PKCE plain refused with a redirect error"         "$(echo "$loc" | grep -c 'error=invalid_request')" 1

# One real login in a cookie jar, stopping before the final hop.
jar=$(mktemp); email="neg$RANDOM@example.cm"
login=$(curl -s -c $jar -o /dev/null -w '%{redirect_url}' "$ISS/authorize?response_type=code&client_id=another-app&scope=openid&redirect_uri=http://127.0.0.1:5555/callback&state=s1&code_challenge=$CH&code_challenge_method=S256")
txn=${login##*/login/}
check "email POST without the txn cookie refused"        "$(curl -s -o /dev/null -w '%{http_code}' -d email=$email "$ISS/login/$txn/email")" 400
curl -s -b $jar -c $jar -o /dev/null -d email=$email "$ISS/login/$txn/email"
sleep 0.5
code=$(curl -s "$MP/api/v1/search?query=to:$email&limit=1" | grep -o '"Subject":"[0-9]\{6\}' | grep -o '[0-9]\{6\}')
for i in 1 2; do curl -s -b $jar -o /dev/null -d code=000000 "$ISS/login/$txn/code"; done
cb=$(curl -s -b $jar -o /dev/null -w '%{redirect_url}' -d "code=$code" "$ISS/login/$txn/code")
check "right code after 2 wrong ones redirects with state+iss" "$(echo "$cb" | grep -c 'state=s1.*iss=\|iss=.*state=s1')" 1
ac=$(echo "$cb" | sed -n 's/.*[?&]code=\([^&]*\).*/\1/p')
tok() { curl -s -o /dev/null -w '%{http_code}' -d grant_type=authorization_code -d client_id=another-app -d redirect_uri=http://127.0.0.1:5555/callback -d "code=$ac" -d "code_verifier=$1" "$ISS/oauth2/token"; }
check "wrong PKCE verifier refused"                      "$(tok wrongwrongwrongwrongwrongwrongwrongwrongwrong)" 400
check "code burnt by the failed attempt"                 "$(tok $VER)" 400
check "txn is single-use"                                "$(curl -s -b $jar -o /dev/null -w '%{http_code}' -d code=$code "$ISS/login/$txn/code")" 410

# Lockout after OTP_MAX_ATTEMPTS wrong codes.
jar2=$(mktemp); email2="lock$RANDOM@example.cm"
login=$(curl -s -c $jar2 -o /dev/null -w '%{redirect_url}' "$ISS/authorize?response_type=code&client_id=another-app&scope=openid&redirect_uri=http://127.0.0.1:5555/callback&code_challenge=$CH&code_challenge_method=S256")
txn=${login##*/login/}
curl -s -b $jar2 -o /dev/null -d email=$email2 "$ISS/login/$txn/email"
for i in 1 2 3 4 5; do curl -s -b $jar2 -o /dev/null -d code=000000 "$ISS/login/$txn/code"; done
check "6th wrong code locks the code"                    "$(curl -s -b $jar2 -o /dev/null -w '%{http_code}' -d code=000000 "$ISS/login/$txn/code")" 429
rm -f $jar $jar2
exit $fail
