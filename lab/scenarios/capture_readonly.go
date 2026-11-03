package scenarios

import (
	"database/sql"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"path/filepath"
)

func captureSQL(endpoint world.Endpoint, query string) int64 {
	db, err := sql.Open("sqlite", "file:"+filepath.Join(endpoint.Home, "state.db")+"?mode=ro&_pragma=busy_timeout(5000)")
	if err != nil {
		return -1
	}
	defer db.Close()
	var value int64
	if db.QueryRow(query).Scan(&value) != nil {
		return -1
	}
	return value
}
