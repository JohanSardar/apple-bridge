package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.timer.start", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.timer.start", Input: Command{}, Output: CommandRecord{}, Description: "Operate app-owned iPhone AlarmKit alarms and timers"}}
	})
	registerAlarmCommand("iphone.timer.start")
}
