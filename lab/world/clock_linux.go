package world

import "golang.org/x/sys/unix"

func UptimeNS() int64 {
	var value unix.Timespec
	if unix.ClockGettime(unix.CLOCK_MONOTONIC, &value) != nil {
		return 0
	}
	return value.Nano()
}
