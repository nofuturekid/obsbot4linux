#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Live check of the readback fields: sets every value of a setting, reads it back through
# `obsbot4linux status` and compares. One line per check; exits 1 if any check FAILs.
# Wakes the camera first and restores all values (and sleep) at the end.
# Needs a connected Tiny 2 Lite, jq, and a built obsbot4linux (OBSBOT4LINUX=/path/to/obsbot4linux to override).
set -u
T=${OBSBOT4LINUX:-"$(dirname "$0")/../target/debug/obsbot4linux"}
t() { "$T" --direct --json "$@"; }
st() { t status; }
fails=0

# chk FIELD SET EXPECTED-JSON JQ-PATH: compares the value read back with the expected one
chk() {
	local actual ok
	actual=$(st | jq -c "$4")
	ok=$(jq -n --argjson a "${actual:-null}" --argjson e "$3" '$a==$e')
	if [ "$ok" = true ]; then
		echo "PASS $1 value=$2 expected=$3 actual=$actual"
	else
		echo "FAIL $1 value=$2 expected=$3 actual=$actual"
		fails=$((fails + 1))
	fi
}
# try FIELD VALUE EXPECTED-JSON JQ-PATH ARGS...: set, wait, check
try() {
	local f=$1 v=$2 e=$3 p=$4
	shift 4
	t "$@" >/dev/null
	sleep 0.5
	chk "$f" "$v" "$e" "$p"
}

snap=$(st) || {
	echo "no camera"
	exit 2
}
asleep=$(jq -r .status.asleep <<<"$snap")
t wake >/dev/null
sleep 3
t tracking static >/dev/null
sleep 1

# 0x0104 gesture switches (and the status block's target lock), both directions
for v in on off on; do
	b=$([ $v = on ] && echo true || echo false)
	try tracking.target_lock $v $b .tracking.target_lock gesture set target-lock $v
	sleep 1.5 # the status block follows the 0x0104 readback about 1 s late
	chk status.gesture_target_lock $v $b .status.gesture_target_lock
	try tracking.zoom_gesture $v $b .tracking.zoom_gesture gesture set zoom $v
	try tracking.dynamic_zoom $v $b .tracking.dynamic_zoom gesture set dynamic-zoom $v
	try tracking.direction_change $v $b .tracking.direction_change gesture set direction-change $v
done
for r in 1.0 1.5 3.0 4.0 2.0; do
	try tracking.gesture_zoom_ratio $r $r .tracking.gesture_zoom_ratio gesture zoom-ratio $r
done

# status block: audio and auto-sleep
for v in on off on; do
	b=$([ $v = on ] && echo true || echo false)
	try status.auto_gain $v $b .status.auto_gain audio gain $v
	try status.mic_in_sleep $v $b .status.mic_in_sleep audio sleep-mic $v
done
for d in near standard far near; do
	try status.pickup_distance $d "\"$d\"" .status.pickup_distance audio distance $d
done
for s in 30 120 600; do
	try status.auto_sleep_seconds $s $s .status.auto_sleep_seconds auto-sleep $s
	chk status.auto_sleep_enabled $s true .status.auto_sleep_enabled
done
try status.auto_sleep_enabled off false .status.auto_sleep_enabled auto-sleep off
chk status.auto_sleep_seconds off 600 .status.auto_sleep_seconds # off keeps the time

# anti-flicker: status byte 0x03 vs V4L2 power_line_frequency
orig_plf=$(jq -r .image.power_line_frequency.value <<<"$snap")
for v in 0 2 1; do
	name=$(case $v in 0) echo off ;; 1) echo 50hz ;; 2) echo 60hz ;; esac)
	try status.anti_flicker $v "\"$name\"" .status.anti_flicker image power_line_frequency $v
done

# AI modes (normal, upper-body, ... stayed static in an empty, dark room; hand and group stuck).
# Status block and 0x0104 must both show the mode; a gesture can switch it, retry once
for m in normal upper-body close-up headless lower-body hand group static; do
	j=${m//-/_}
	for attempt in 1 2; do
		t tracking $m >/dev/null
		sleep 2
		now=$(st | jq -c '[.status.ai_mode, .tracking.ai_mode]')
		[ "$now" = "[\"$j\",\"$j\"]" ] && break
	done
	chk ai_mode "$m" "[\"$j\",\"$j\"]" '[.status.ai_mode, .tracking.ai_mode]'
done

# restore the snapshot
v() { jq -r "$1" <<<"$snap"; }
onoff() { [ "$1" = true ] && echo on || echo off; }
t gesture set target-lock "$(onoff "$(v .tracking.target_lock)")" >/dev/null
t gesture set zoom "$(onoff "$(v .tracking.zoom_gesture)")" >/dev/null
t gesture set dynamic-zoom "$(onoff "$(v .tracking.dynamic_zoom)")" >/dev/null
t gesture set direction-change "$(onoff "$(v .tracking.direction_change)")" >/dev/null
t gesture zoom-ratio "$(v .tracking.gesture_zoom_ratio)" >/dev/null
t audio gain "$(onoff "$(v .status.auto_gain)")" >/dev/null
t audio sleep-mic "$(onoff "$(v .status.mic_in_sleep)")" >/dev/null
t audio distance "$(v .status.pickup_distance)" >/dev/null
t auto-sleep "$(v .status.auto_sleep_seconds)" >/dev/null
[ "$(v .status.auto_sleep_enabled)" = true ] || {
	sleep 0.5
	t auto-sleep off >/dev/null
}
t image power_line_frequency "$orig_plf" >/dev/null
t tracking static >/dev/null
sleep 1
t gimbal --pan 0 --tilt 0 >/dev/null
[ "$asleep" = true ] && {
	sleep 2
	t sleep >/dev/null
}

echo "$fails check(s) failed"
[ "$fails" -eq 0 ]
