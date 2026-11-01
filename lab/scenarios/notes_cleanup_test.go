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
				want := map[string]any{"source": "notes", "verify_deleted_ids": []string{id}}
				if action == "cleanup_owned" {
					want["permanent"] = true
				}
				if !reflect.DeepEqual(args, want) {
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

func TestNotesCleanupFoldersBatchesOnlyCapturedIdentities(t *testing.T) {
	for _, delayed := range []bool{false, true} {
		fixtureCalls, observeCalls := 0, 0
		err := notesCleanupFolders(func(verb string, request map[string]any) (map[string]any, error) {
			if verb == "fixture" {
				fixtureCalls++
				want := map[string]any{"source": "notes", "tag": "[ab-test owned]", "action": "cleanup_created_folders", "created_by_run": true, "folders": []map[string]any{{"id": "a", "title": "Bridge Test [ab-test owned] a"}, {"id": "b", "title": "Bridge Test [ab-test owned] b"}}}
				if !reflect.DeepEqual(request, want) {
					t.Fatalf("cleanup widened captured folders: %#v", request)
				}
				if delayed {
					return nil, errors.New("notestore_deletion_unverified")
				}
				return map[string]any{"status": "verified"}, nil
			}
			observeCalls++
			if !reflect.DeepEqual(request, map[string]any{"source": "notes", "verify_deleted_ids": []string{"a", "b"}}) {
				t.Fatalf("verification changed captured identities: %#v", request)
			}
			return map[string]any{"status": "verified", "deletion_verified": true}, nil
		}, "[ab-test owned]", map[string]string{"b": "Bridge Test [ab-test owned] b", "a": "Bridge Test [ab-test owned] a"})
		if err != nil || fixtureCalls != 1 || observeCalls != map[bool]int{false: 0, true: 1}[delayed] {
			t.Fatalf("err=%v deletes=%d observes=%d", err, fixtureCalls, observeCalls)
		}
	}
}

func TestNotesCleanupFoldersRequiresEveryDeletion(t *testing.T) {
	problem := errors.New("notestore_deletion_unverified")
	err := notesCleanupTruth(func(verb string, request map[string]any) (map[string]any, error) {
		if verb == "fixture" {
			return nil, problem
		}
		return map[string]any{"status": "verified", "deletion_verified": false}, nil
	}, 0, map[string]any{"action": "cleanup_created_folders", "folders": []map[string]any{{"id": "a"}, {"id": "b"}}})
	if !errors.Is(err, problem) {
		t.Fatalf("partial deletion accepted: %v", err)
	}
}
