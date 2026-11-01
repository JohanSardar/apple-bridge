package scenarios

import "time"

func notesCleanupTruth(recover func(string, map[string]any) (map[string]any, error), settle time.Duration, request map[string]any) error {
	_, problem := recover("fixture", request)
	if problem == nil {
		return nil
	}
	key := "id"
	if request["action"] == "cleanup_created_folder" {
		key = "folder_id"
	}
	id, _ := request[key].(string)
	if id == "" {
		return problem
	}
	deadline := time.Now().Add(settle)
	for {
		response, err := recover("observe", map[string]any{"source": "notes", "verify_deleted_ids": []string{id}})
		if err == nil && response["status"] == "verified" && response["deletion_verified"] == true {
			return nil
		}
		if !time.Now().Before(deadline) {
			return problem
		}
		time.Sleep(min(250*time.Millisecond, time.Until(deadline)))
	}
}
