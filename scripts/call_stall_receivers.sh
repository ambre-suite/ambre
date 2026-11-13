#!/usr/bin/env bash
#
# The receivers' machine of the call video stall proof
# (specs/1-Foundations/ambre-spec-call-proofs.md, the stall proof).
#
# Run it as root on the machine of the four browsers, before `just call-stall-proof` on the
# machine of the SFU:
#
#   sudo scripts/call_stall_receivers.sh <address of the SFU machine> [interface]
#
# It does three things, and undoes the first two on exit (Ctrl-C):
#
# 1. It impairs the UDP between this machine and the SFU machine with tc netem: 75 ms of
#    delay and 2 % of loss each way, so 150 ms of round trip, as the budget row states. Other
#    traffic, the WebDriver commands and the signalling included, passes untouched. The
#    traffic from the SFU machine reaches netem through an ifb device.
# 2. It starts two chromedriver and two geckodriver servers on the address of this machine,
#    as the user who called sudo. A geckodriver serves one session, so each Firefox has its own.
# 3. It prints the LOAD_DRIVERS line to give `just call-stall-proof`, then holds.
#
# Variables: CHROMEDRIVER and GECKODRIVER name the drivers (default: on the PATH).
# STALL_DELAY and STALL_LOSS change the impairment of each direction (default 75ms and 2%).
# STALL_DRY_RUN=1 prints each command instead of running it, and does not hold.

set -euo pipefail

SFU_HOST="${1:?usage: call_stall_receivers.sh <address of the SFU machine> [interface]}"
ROUTE="$(ip route get "$SFU_HOST")"
IFACE="${2:-$(sed -n 's/.* dev \([^ ]*\).*/\1/p' <<< "$ROUTE" | head -n 1)}"
SELF="$(sed -n 's/.* src \([^ ]*\).*/\1/p' <<< "$ROUTE" | head -n 1)"
DELAY="${STALL_DELAY:-75ms}"
LOSS="${STALL_LOSS:-2%}"
CHROMEDRIVER="${CHROMEDRIVER:-chromedriver}"
GECKODRIVER="${GECKODRIVER:-geckodriver}"
DRY="${STALL_DRY_RUN:-}"
IFB="ifb-stall"
PIDS=()

run() {
    if [ -n "$DRY" ]; then echo "$*"; else "$@"; fi
}

# Starts one driver in the background, as the user who called sudo: a browser of root refuses
# its sandbox.
launch() {
    if [ -n "$DRY" ]; then
        echo "$*"
    elif [ -n "${SUDO_USER:-}" ]; then
        sudo -u "$SUDO_USER" "$@" &
        PIDS+=("$!")
    else
        "$@" &
        PIDS+=("$!")
    fi
}

netem_up() {
    run modprobe ifb 2> /dev/null || true
    run ip link add "$IFB" type ifb
    run ip link set "$IFB" up
    # Out: the UDP to the SFU machine takes band 4, which netem holds.
    run tc qdisc add dev "$IFACE" root handle 1: prio bands 4 priomap 1 2 2 2 1 2 0 0 1 1 1 1 1 1 1 1
    run tc qdisc add dev "$IFACE" parent 1:4 handle 40: netem delay "$DELAY" loss "$LOSS" limit 10000
    run tc filter add dev "$IFACE" parent 1:0 protocol ip prio 1 u32 \
        match ip dst "$SFU_HOST/32" match ip protocol 17 0xff flowid 1:4
    # In: the UDP from the SFU machine goes to the ifb device, whose netem holds it.
    run tc qdisc add dev "$IFACE" handle ffff: ingress
    run tc filter add dev "$IFACE" parent ffff: protocol ip prio 1 u32 \
        match ip src "$SFU_HOST/32" match ip protocol 17 0xff \
        action mirred egress redirect dev "$IFB"
    run tc qdisc add dev "$IFB" root netem delay "$DELAY" loss "$LOSS" limit 10000
}

cleanup() {
    echo "call_stall_receivers: what netem held, each way:"
    tc -s qdisc show dev "$IFACE" | sed -n '/netem/,+2p' || true
    tc -s qdisc show dev "$IFB" | sed -n '/netem/,+2p' || true
    for pid in "${PIDS[@]}"; do kill "$pid" 2> /dev/null || true; done
    tc qdisc del dev "$IFACE" root 2> /dev/null || true
    tc qdisc del dev "$IFACE" ingress 2> /dev/null || true
    ip link del "$IFB" 2> /dev/null || true
}

start_drivers() {
    launch "$CHROMEDRIVER" --port=9515 --allowed-ips="$SFU_HOST"
    launch "$CHROMEDRIVER" --port=9516 --allowed-ips="$SFU_HOST"
    launch "$GECKODRIVER" --host "$SELF" --port 4444 --websocket-port 9233
    launch "$GECKODRIVER" --host "$SELF" --port 4445 --websocket-port 9234
}

echo "call_stall_receivers: SFU machine $SFU_HOST, this machine $SELF on $IFACE,"
echo "call_stall_receivers: $DELAY of delay and $LOSS of loss on the UDP each way"
if [ -z "$DRY" ]; then
    [ "$(id -u)" -eq 0 ] || { echo "run it as root: tc needs it" >&2; exit 1; }
    trap cleanup EXIT
    trap 'exit 130' INT TERM
fi
netem_up
start_drivers
[ -n "$DRY" ] || sleep 2
echo "call_stall_receivers: on the SFU machine, run:"
echo "LOAD_DRIVERS=chrome@$SELF:9515,chrome@$SELF:9516,firefox@$SELF:4444,firefox@$SELF:4445 just call-stall-proof <address of the SFU machine>"
[ -n "$DRY" ] || wait
