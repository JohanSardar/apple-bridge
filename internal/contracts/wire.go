package contracts

import "encoding/json"

type ipcFields IPCFrame

func (f IPCFrame) MarshalJSON() ([]byte, error) {
	switch f.T {
	case "Query":
		return json.Marshal(struct {
			T string `json:"t"`
			Query
		}{f.T, Query{ID: f.ID, Op: f.Op, Args: f.Args, DeadlineMS: f.DeadlineMS}})
	case "QueryResult":
		blobs := f.Blobs
		if blobs == nil {
			blobs = []QueryBlob{}
		}
		return MarshalQueryJSON(struct {
			T string `json:"t"`
			QueryResult
		}{f.T, QueryResult{ID: f.ID, ResultJSON: f.ResultJSON, Blobs: blobs}})
	case "QueryError":
		return json.Marshal(struct {
			T string `json:"t"`
			QueryError
		}{f.T, QueryError{ID: f.ID, Code: f.Code}})
	case "QueryCancel":
		return json.Marshal(struct {
			T  string `json:"t"`
			ID string `json:"id"`
		}{f.T, f.ID})
	}

	if f.T == "Capture" && f.Capture != nil {
		return json.Marshal(struct {
			T string `json:"t"`
			*Capture
		}{T: f.T, Capture: f.Capture})
	}
	return json.Marshal(ipcFields(f))
}
func (f *IPCFrame) UnmarshalJSON(b []byte) error {
	var fields ipcFields
	if err := json.Unmarshal(b, &fields); err != nil {
		return err
	}
	*f = IPCFrame(fields)
	if f.T == "Capture" {
		var capture Capture
		if err := json.Unmarshal(b, &capture); err != nil {
			return err
		}
		f.Capture = &capture
	}
	return nil
}

type feedFields FeedFrame

func (f FeedFrame) MarshalJSON() ([]byte, error) {
	if f.T == "CommandResponse" {
		return MarshalQueryJSON(feedFields(f))
	}
	if f.T == "Batch" && f.Batch != nil {
		return json.Marshal(struct {
			T string `json:"t"`
			*Batch
		}{T: f.T, Batch: f.Batch})
	}
	return json.Marshal(feedFields(f))
}
func (f *FeedFrame) UnmarshalJSON(b []byte) error {
	var fields feedFields
	if err := json.Unmarshal(b, &fields); err != nil {
		return err
	}
	*f = FeedFrame(fields)
	if f.T == "Batch" {
		var batch Batch
		if err := json.Unmarshal(b, &batch); err != nil {
			return err
		}
		f.Batch = &batch
	}
	return nil
}
