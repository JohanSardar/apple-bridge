package contracts

type ThreadWindow struct {
	ThreadOriginatorID string
	ChatID             string
	First              Record
	Last               Record
}
type WindowCounts struct {
	Older uint64 `json:"older"`
	Newer uint64 `json:"newer"`
}
