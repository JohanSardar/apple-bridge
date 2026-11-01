package scenarios

import (
	"errors"
	"reflect"
	"testing"
	"time"
)

func TestNotesCleanupVerifiesExactIdentityWithoutDeletingAgain(t *testing.T) {
	for _, action := range []string{"cleanup_owned", "cleanup_created_folder"} {
		t.Run(action, func(t *testing.T) {
			calls, observes := 0, 0
			request := map[string]any{"action": action, "id": "owned-note", "folder_id": "owned-folder"}
			problem := errors.New("notestore_deletion_unverified")
			err := notesCleanupTruth(func(verb string, args map[string]any) (map[string]any, error) {
				if verb == "fixture" {
					calls++
					return nil, problem
				}
				observes++
				id := "owned-note"
				if action == "cleanup_created_folder" {
					id = "owned-folder"
				}
				if !reflect.DeepEqual(args, map[string]any{"source": "notes", "verify_deleted_ids": []string{id}}) {
					t.Fatalf("verification widened identity: %#v", args)
				}
				return map[string]any{"status": "verified", "deletion_verified": observes > 1}, nil
			}, time.Second, request)
			if err != nil || calls != 1 || observes != 2 {
				t.Fatalf("err=%v deletes=%d observes=%d", err, calls, observes)
			}
		})
	}
}

func TestNotesCleanupPreservesUnverifiedFailure(t *testing.T) {
	for _, response := range []map[string]any{{"status": "verified", "deletion_verified": false}, {"deletion_verified": true}} {
		problem := errors.New("cleanup failed")
		calls := 0
		err := notesCleanupTruth(func(verb string, args map[string]any) (map[string]any, error) {
			calls++
			if verb == "fixture" {
				return nil, problem
			}
			return response, nil
		}, 0, map[string]any{"action": "cleanup_owned", "id": "owned-note"})
		if !errors.Is(err, problem) || calls != 2 {
			t.Fatalf("err=%v calls=%d", err, calls)
		}
	}
}

func TestNotesCleanupWithoutIdentityDoesNotBroadenVerification(t *testing.T) {
	problem := errors.New("cleanup failed")
	calls := 0
	err := notesCleanupTruth(func(string, map[string]any) (map[string]any, error) {
		calls++
		return nil, problem
	}, time.Second, map[string]any{"action": "cleanup_created_folder", "native_key": "folder-key"})
	if !errors.Is(err, problem) || calls != 1 {
		t.Fatalf("err=%v calls=%d", err, calls)
	}
}
