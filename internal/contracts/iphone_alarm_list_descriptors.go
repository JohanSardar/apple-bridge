package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.alarm.list", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.alarm.list", Input: Command{}, Output: CommandRecord{}, Description: "Operate app-owned iPhone AlarmKit alarms and timers"}}
	})
	registerAlarmCommand("iphone.alarm.list")
}
