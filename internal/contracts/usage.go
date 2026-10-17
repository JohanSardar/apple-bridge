package contracts

type UsageQuery struct {
    DeviceID string `json:"device_id,omitempty"`
    FromDay string `json:"from_day,omitempty"`
    ThroughDay string `json:"through_day,omitempty"`
    BundleID string `json:"bundle_id,omitempty"`
    After string `json:"after,omitempty"`
    Limit int `json:"limit,omitempty"`
}
