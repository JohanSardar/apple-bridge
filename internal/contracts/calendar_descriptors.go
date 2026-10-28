package contracts

func CalendarDescriptors() []OperationDescriptor {
	result := []OperationDescriptor{{Name: "calendar.agenda", Version: 1, Kind: ReadOp, ReadModel: true, Source: Calendar, Scope: "calendar:read", Input: AgendaQuery{}, Output: ReadResult{}, Description: "Read local occurrences overlapping a time range; all-day dates remain floating"}}
	result = append(result, OperationDescriptor{Name: "calendar.range", Version: 1, Kind: ReadOp, ReadModel: true, Source: Calendar, Scope: "calendar:read", Input: CalendarRangeQuery{}, Output: CalendarRangeResult{}, Description: "Read a half-open Calendar range with floating dates, calendar filters and precomputed overlap groups"})
	for _, op := range []string{"create_event", "update_event", "delete_event", "create_calendar"} {
		name := "calendar." + op
		result = append(result, OperationDescriptor{Name: name, Version: 1, Kind: CommandOp, Source: Calendar, Scope: "calendar:write", Capability: name, Input: Command{}, Output: CommandRecord{}, Description: "Calendar " + op + "; update and delete require expected_version and span this or future"})
	}
	return result
}

func init() { registerDescriptors(CalendarDescriptors, 0, 1, 2, 3, 4, 5) }
