#!/bin/sh
tc qdisc add dev eth0 root netem delay 800ms
exec socat TCP-LISTEN:2225,fork,reuseaddr TCP:dalan-test-ssh:2222
