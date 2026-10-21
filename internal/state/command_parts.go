package state

import "github.com/JohanSardar/apple-bridge/internal/contracts"

func commandPartsChange(old, next []contracts.CommandPart) (bool, bool) {
	if len(next) > contracts.MessageFileCount+1 || len(old) > 0 && len(old) != len(next) {
		return false, false
	}
	changed := len(old) != len(next)
	ids := map[string]bool{}
	for i, part := range next {
		if part.Index != i || part.Kind != "text" && part.Kind != "attachment" || part.Kind == "text" && part.UploadID != "" {
			return false, false
		}
		switch part.Disposition {
		case "observed", "delivered", "delivery_failed":
		case "indeterminate", "failed_no_effect":
			if part.RecordID != "" {
				return false, false
			}
		default:
			return false, false
		}
		if part.RecordID != "" {
			if ids[part.RecordID] {
				return false, false
			}
			ids[part.RecordID] = true
		}
		if len(old) == 0 {
			continue
		}
		prior := old[i]
		if prior.Index != part.Index || prior.Kind != part.Kind || prior.UploadID != part.UploadID || prior.RecordID != "" && prior.RecordID != part.RecordID {
			return false, false
		}
		if prior.Disposition != part.Disposition && prior.Disposition != "indeterminate" && !(prior.Disposition == "observed" && (part.Disposition == "delivered" || part.Disposition == "delivery_failed")) {
			return false, false
		}
		changed = changed || prior != part
	}
	return changed, true
}
