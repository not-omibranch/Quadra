#!/usr/bin/env bash
# End-to-end check of the Android build on an emulator (run by .github/workflows/android.yml).
# A VLESS server is started on the host; the app (a debug build) is told through its autotest
# file to add that server, connect, stay connected for a while and disconnect. While it is
# connected another program on the phone makes a request, and the server's log has to show it:
# that proves the whole chain, system VPN -> tun2proxy -> the core on the phone -> the server.
# A request to a domain in the exclusions has to work too, and go past the server: that is the
# core resolving a name on its own, through the system's resolver.
# Then the app connects once more with that other program in its exclusions, and this time the
# same request must go past the server.
#   tools/android-e2e.sh <apk>
# Environment:
#   TRANSPORT=tcp|xhttp   the server's transport; xhttp is XHTTP over REALITY, as the fleet runs it
#   CORE=auto|xray        the app's "Ядро" setting; auto must connect with sing-box
#   SHORT=1               skip the second connection with a program excluded
#   OUT=<dir>             where screenshots and logs go (android-e2e)
set -u
APK="$1"
TRANSPORT="${TRANSPORT:-tcp}"
CORE="${CORE:-auto}"
SHORT="${SHORT:-0}"
PKG=dev.quadra.client
OUT="${OUT:-android-e2e}"
UUID=5783a3e7-e373-51cd-8642-c83782b807c5
PORT=24443
HOLD=45
# the program whose request is watched: the shell user's package
OTHER=com.android.shell
# a domain in the exclusions: goes direct, so the core on the phone resolves it itself
DIRECT=example.org
case "$CORE" in xray) ENGINE=Xray ;; *) ENGINE=sing-box ;; esac
mkdir -p "$OUT"
echo "== scenario: transport $TRANSPORT, core setting $CORE (expect $ENGINE)"

# one plain HTTP request from the shell user, which is not our app and so goes through the VPN
request() {
  local host="${1:-example.com}"
  adb shell "{ printf 'GET / HTTP/1.0\r\nHost: $host\r\n\r\n'; sleep 6; } | nc -w 12 $host 80 | head -1" | tr -d '\r'
}
read_app_file() {
  adb shell "run-as $PKG sh -c 'cat files/$1 $1 app_data/$1 2>/dev/null'" | tr -d '\r'
}
seen() { grep -c "$1:80" "$OUT/server-access.log"; }

echo "== test server on the host"
[ -x src-tauri/resources/xray/xray ] || node tools/fetch-xray.mjs >/dev/null
XRAY=src-tauri/resources/xray/xray
if [ "$TRANSPORT" = xhttp ]; then
  KEYS=$($XRAY x25519)
  PRIVATE=$(echo "$KEYS" | sed -n 1p | sed 's/.*: *//')
  PUBLIC=$(echo "$KEYS" | sed -n 2p | sed 's/.*: *//')
  SID=6ba85179e30d4fc2
  SNI=www.microsoft.com
  STREAM="{\"network\": \"xhttp\", \"security\": \"reality\",
    \"realitySettings\": {\"target\": \"$SNI:443\", \"serverNames\": [\"$SNI\"], \"privateKey\": \"$PRIVATE\", \"shortIds\": [\"$SID\"]},
    \"xhttpSettings\": {\"path\": \"/qx\", \"mode\": \"auto\"}}"
  LINK="vless://$UUID@10.0.2.2:$PORT?type=xhttp&security=reality&sni=$SNI&fp=chrome&pbk=$PUBLIC&sid=$SID&path=%2Fqx&mode=auto&encryption=none#Test"
else
  STREAM='{"network": "tcp", "security": "none"}'
  LINK="vless://$UUID@10.0.2.2:$PORT?type=tcp&security=none&encryption=none#Test"
fi
cat > "$OUT/server.json" <<EOF
{
  "log": {"loglevel": "info", "access": "$PWD/$OUT/server-access.log"},
  "inbounds": [{"listen": "0.0.0.0", "port": $PORT, "protocol": "vless",
    "settings": {"clients": [{"id": "$UUID"}], "decryption": "none"},
    "streamSettings": $STREAM}],
  "outbounds": [{"protocol": "freedom"}]
}
EOF
: > "$OUT/server-access.log"
$XRAY run -c "$OUT/server.json" > "$OUT/server.log" 2>&1 &
SERVER=$!
sleep 2
kill -0 $SERVER || { echo "server did not start"; cat "$OUT/server.log"; exit 1; }

echo "== install"
adb uninstall $PKG >/dev/null 2>&1 || true
adb install -r "$APK" || exit 1
# the consent dialog needs a finger; this is the same switch it flips
adb shell appops set $PKG ACTIVATE_VPN allow
adb shell pm grant $PKG android.permission.POST_NOTIFICATIONS 2>/dev/null || true

echo "== autotest file"
EXCLUDE=""
[ "$SHORT" = 1 ] || EXCLUDE=",\"exclude\":[\"$OTHER\"],\"pause\":30"
JOB="{\"link\":\"$LINK\",\"connect\":true,\"hold\":$HOLD$EXCLUDE,\"settings\":{\"core\":\"$CORE\",\"bypass_domains\":[\"$DIRECT\"]}}"
for dir in files . app_data; do
  adb shell "run-as $PKG mkdir -p $dir" 2>/dev/null
  echo "$JOB" | adb shell "run-as $PKG sh -c 'cat > $dir/autotest.json'"
done

echo "== start"
adb logcat -c
adb shell am start -n $PKG/.MainActivity
sleep 10
adb exec-out screencap -p > "$OUT/1-started.png"

echo "== wait for the connection"
RESULT=""
for i in $(seq 1 30); do
  RESULT=$(read_app_file autotest-result.json)
  case "$RESULT" in *'"stage":"connect"'*) break ;; esac
  sleep 2
done
echo "autotest result: $RESULT"
echo "$RESULT" > "$OUT/result.json"
adb exec-out screencap -p > "$OUT/2-connected.png"

echo "== the system's view"
adb shell dumpsys connectivity 2>/dev/null | grep -oE "Transports: [A-Z|]*VPN[A-Z|]*|sessionId=[A-Za-z]*" | sort -u | head -4
TUN_UP=$(adb shell ip addr 2>/dev/null | grep -cE "^[0-9]+: tun[0-9]")
CORES=$(adb shell "ps -A -o NAME 2>/dev/null" | tr -d '\r' | grep -E "libsingbox|libxray" | sort -u | tr '\n' ' ')
echo "core processes: $CORES"

echo "== a request from another program on the phone, through the tunnel"
REPLY=$(request)
echo "reply: $REPLY"
adb exec-out screencap -p > "$OUT/3-traffic.png"

echo "== a request to $DIRECT, which is in the exclusions"
REPLY_BYPASS=$(request $DIRECT)
echo "reply: $REPLY_BYPASS"
BYPASS_LINES=$(seen $DIRECT)

echo "== wait for the app to disconnect again"
GONE=""
for i in $(seq 1 40); do
  GONE=$(read_app_file autotest-disconnect.json)
  [ -n "$GONE" ] && break
  sleep 2
done
echo "after disconnect: $GONE"
sleep 3
TUN_AFTER=$(adb shell ip addr 2>/dev/null | grep -cE "^[0-9]+: tun[0-9]")
VPN_AFTER=$(adb shell dumpsys connectivity 2>/dev/null | grep -cE "sessionId=Quadra")
CORES_AFTER=$(adb shell "ps -A -o NAME 2>/dev/null" | tr -d '\r' | grep -cE "libsingbox|libxray")
BEFORE_LINES=$(seen example.com)
REPLY_DIRECT=$(request)
AFTER_LINES=$(seen example.com)
echo "direct reply: $REPLY_DIRECT"
adb exec-out screencap -p > "$OUT/4-disconnected.png"

if [ "$SHORT" != 1 ]; then
  echo "== wait for the second connection, with $OTHER excluded"
  EXCLUDED=""
  for i in $(seq 1 30); do
    EXCLUDED=$(read_app_file autotest-excluded.json)
    [ -n "$EXCLUDED" ] && break
    sleep 2
  done
  echo "second connection: $EXCLUDED"
  TUN_EXCLUDED=$(adb shell ip addr 2>/dev/null | grep -cE "^[0-9]+: tun[0-9]")
  REPLY_EXCLUDED=$(request)
  EXCLUDED_LINES=$(seen example.com)
  echo "reply with the program excluded: $REPLY_EXCLUDED"
  adb exec-out screencap -p > "$OUT/5-excluded.png"
fi

echo "== what the server saw"
tail -12 "$OUT/server-access.log" | cut -c1-140

echo "== app log"
read_app_file core.log > "$OUT/core.log"
tail -30 "$OUT/core.log"
adb logcat -d > "$OUT/logcat.txt" 2>/dev/null
grep -E "AndroidRuntime|FATAL EXCEPTION|panicked" "$OUT/logcat.txt" | tail -20

kill $SERVER 2>/dev/null

FAIL=0
check() { if [ "$1" = ok ]; then echo "PASS: $2"; else echo "FAIL: $2"; FAIL=1; fi; }
case "$RESULT" in *'"state":"on"'*) check ok "the app reports connected" ;; *) check no "the app reports connected" ;; esac
grep -q -- "--- подключение: .*ядро $ENGINE" "$OUT/core.log" && check ok "the connection was made by $ENGINE" || check no "the connection was made by $ENGINE"
case "$ENGINE:$CORES" in sing-box:*libsingbox*|Xray:*libxray*) check ok "its process runs ($CORES)" ;; *) check no "its process runs ($CORES)" ;; esac
[ "$TUN_UP" -ge 1 ] && check ok "the tunnel device exists while connected" || check no "the tunnel device exists while connected"
[ "$BEFORE_LINES" -ge 1 ] && check ok "another program's request reached the server through the tunnel" || check no "another program's request reached the server through the tunnel"
case "$REPLY" in HTTP/*) check ok "and it got an answer ($REPLY)" ;; *) check no "and it got an answer" ;; esac
case "$REPLY_BYPASS" in HTTP/*) check ok "a domain in the exclusions answers ($REPLY_BYPASS)" ;; *) check no "a domain in the exclusions answers" ;; esac
[ "$BYPASS_LINES" -eq 0 ] && check ok "and it goes past the server" || check no "and it goes past the server (server saw it $BYPASS_LINES times)"
case "$GONE" in *'"state":"off"'*) check ok "the app reports disconnected" ;; *) check no "the app reports disconnected" ;; esac
[ "$TUN_AFTER" -eq 0 ] && [ "$VPN_AFTER" -eq 0 ] && check ok "the system VPN is gone after disconnecting" || check no "the system VPN is gone after disconnecting (tun=$TUN_AFTER vpn=$VPN_AFTER)"
[ "$CORES_AFTER" -eq 0 ] && check ok "and so is the core's process" || check no "and so is the core's process ($CORES_AFTER left)"
case "$REPLY_DIRECT" in HTTP/*) check ok "the network still works afterwards ($REPLY_DIRECT)" ;; *) check no "the network still works afterwards" ;; esac
[ "$AFTER_LINES" -eq "$BEFORE_LINES" ] && check ok "and that request no longer goes through the server" || check no "and that request no longer goes through the server"
case "$RESULT" in *'"apps":null'*|*'"apps":0'*) check no "the app can list the installed programs" ;; *'"apps":'*) check ok "the app can list the installed programs" ;; *) check no "the app can list the installed programs" ;; esac
if [ "$SHORT" != 1 ]; then
  case "$EXCLUDED" in *'"state":"on"'*) check ok "connected again with a program excluded" ;; *) check no "connected again with a program excluded" ;; esac
  [ "$TUN_EXCLUDED" -ge 1 ] && check ok "the tunnel device exists again" || check no "the tunnel device exists again"
  case "$REPLY_EXCLUDED" in HTTP/*) check ok "the excluded program still gets an answer ($REPLY_EXCLUDED)" ;; *) check no "the excluded program still gets an answer" ;; esac
  [ "$EXCLUDED_LINES" -eq "$AFTER_LINES" ] && check ok "and its request goes past the server" || check no "and its request goes past the server (server saw $EXCLUDED_LINES, was $AFTER_LINES)"
fi
exit $FAIL
