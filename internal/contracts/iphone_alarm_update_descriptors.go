package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.alarm.update", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.alarm.update", Input: Command{}, Output: CommandRecord{}, Description: "Operate app-owned iPhone AlarmKit alarms and timers"}}
	})
	registerAlarmCommand("iphone.alarm.update")
}
