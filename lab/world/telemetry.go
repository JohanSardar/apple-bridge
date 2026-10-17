package world

import (
	"context"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"time"
)

type HostSample struct {
	At              time.Time `json:"at"`
	CPUCount        int       `json:"cpu_count"`
	Load            []float64 `json:"load_average"`
	CPUIdle         *float64  `json:"cpu_idle_percent"`
	FreeMemory      *uint64   `json:"free_memory_bytes"`
	HeavySlots      int       `json:"active_heavy_slots"`
	OtherHeavySlots int       `json:"other_active_heavy_slots"`
	QueueLength     int       `json:"heavy_queue_length"`
	Quiet           bool      `json:"quiet"`
	Unavailable     []string  `json:"unavailable,omitempty"`
}

const telemetryProbe = `
if [ "$(uname)" = Darwin ]; then
  echo "cpus $(sysctl -n hw.logicalcpu)"
  echo "load $(sysctl -n vm.loadavg | tr -d '{}')"
  top -l 2 -s 1 -n 0 | awk '/CPU usage/ {idle=$7} END {gsub(/%/, "", idle); print "idle " idle}'
  vm_stat | awk '/page size of/ {size=$8} /Pages free:/ {gsub(/\./,"",$3); free=$3} END {if(size>0) printf "memory %.0f\n",size*free}'
else
  echo "cpus $(getconf _NPROCESSORS_ONLN)"
  awk '{print "load " $1 " " $2 " " $3}' /proc/loadavg
  read label u n s idle wait irq soft steal rest < /proc/stat
  first_idle=$idle; first_total=$((u+n+s+idle+wait+irq+soft+steal))
  sleep 1
  read label u n s idle wait irq soft steal rest < /proc/stat
  awk -v a="$first_idle" -v b="$first_total" -v c="$idle" -v d="$((u+n+s+idle+wait+irq+soft+steal))" 'BEGIN {if(d>b) print "idle " 100*(c-a)/(d-b)}'
  awk '/^MemFree:/ {printf "memory %.0f\n",$2*1024}' /proc/meminfo
fi
ancestors=" "
current=$$
while [ "$current" -gt 1 ] 2>/dev/null; do
 ancestors="$ancestors$current "
 current=$(ps -p "$current" -o ppid= | tr -d ' ')
done
active=0; other=0; queued=0
for lock in /tmp/apple-bridge-heavy-*.lock/pid; do
 [ -f "$lock" ] || continue
 pid=$(cat "$lock")
 kill -0 "$pid" 2>/dev/null || continue
 active=$((active+1))
 case "$ancestors" in *" $pid "*) ;; *) other=$((other+1));; esac
done
for ticket in /tmp/apple-bridge-heavy-queue/*; do
 [ -f "$ticket" ] || continue
 pid=${ticket##*-}
 kill -0 "$pid" 2>/dev/null && queued=$((queued+1))
done
printf 'slots %s\nother %s\nqueue %s\n' "$active" "$other" "$queued"
`

func SampleHost(ctx context.Context, remote bool) HostSample {
	sample := HostSample{At: time.Now().UTC()}
	bounded, cancel := context.WithTimeout(ctx, 8*time.Second)
	defer cancel()
	cmd := exec.CommandContext(bounded, "sh", "-s")
	if remote {
		cmd = exec.CommandContext(bounded, "ssh", "-o", "ControlPath=none", "-o", "ConnectTimeout=4", "ubuntu", "sh -s")
	}
	cmd.Stdin = strings.NewReader(telemetryProbe)
	raw, err := cmd.Output()
	if err != nil {
		sample.Unavailable = append(sample.Unavailable, "host_probe_failed")
	}
	fields := map[string][]string{}
	for _, line := range strings.Split(string(raw), "\n") {
		values := strings.Fields(line)
		if len(values) > 1 {
			fields[values[0]] = values[1:]
		}
	}
	integer := func(key string) int {
		if len(fields[key]) != 1 {
			sample.Unavailable = append(sample.Unavailable, key)
			return 0
		}
		value, err := strconv.Atoi(fields[key][0])
		if err != nil {
			sample.Unavailable = append(sample.Unavailable, key)
		}
		return value
	}
	sample.CPUCount = integer("cpus")
	sample.HeavySlots = integer("slots")
	sample.OtherHeavySlots = integer("other")
	sample.QueueLength = integer("queue")
	for _, raw := range fields["load"] {
		value, err := strconv.ParseFloat(raw, 64)
		if err == nil {
			sample.Load = append(sample.Load, value)
		}
	}
	if len(sample.Load) != 3 {
		sample.Unavailable = append(sample.Unavailable, "load")
	}
	if len(fields["idle"]) == 1 {
		value, err := strconv.ParseFloat(regexp.MustCompile(`[^0-9.]`).ReplaceAllString(fields["idle"][0], ""), 64)
		if err == nil && value >= 0 && value <= 100 {
			sample.CPUIdle = &value
		}
	}
	if sample.CPUIdle == nil {
		sample.Unavailable = append(sample.Unavailable, "cpu_idle")
	}
	if len(fields["memory"]) == 1 {
		value, err := strconv.ParseUint(fields["memory"][0], 10, 64)
		if err == nil {
			sample.FreeMemory = &value
		}
	}
	if sample.FreeMemory == nil {
		sample.Unavailable = append(sample.Unavailable, "free_memory")
	}
	sample.Quiet = len(sample.Unavailable) == 0 && sample.OtherHeavySlots == 0 && len(sample.Load) == 3 && sample.Load[0] < float64(sample.CPUCount)
	return sample
}
