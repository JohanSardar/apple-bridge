package contracts

import (
	"math"
	"strings"
)

type IPhoneGeofence struct {
	DeviceID  string   `json:"device_id"`
	Name      string   `json:"name"`
	Latitude  *float64 `json:"latitude"`
	Longitude *float64 `json:"longitude"`
	Radius    *float64 `json:"radius"`
}

type IPhoneGeofenceName struct {
	DeviceID string `json:"device_id"`
	Name     string `json:"name"`
}

type IPhoneSignificant struct {
	DeviceID string `json:"device_id"`
	Enabled  *bool  `json:"enabled"`
}

func validFenceName(name string) bool { return strings.TrimSpace(name) != "" && len(name) <= 100 }

func validateIPhoneFence(raw []byte) error {
	var args IPhoneGeofence
	if DecodeInput(raw, &args) != nil || !validFenceName(args.Name) || args.Latitude == nil || args.Longitude == nil || args.Radius == nil {
		return ErrInvalid
	}
	if math.IsNaN(*args.Latitude) || math.IsNaN(*args.Longitude) || math.IsNaN(*args.Radius) || math.Abs(*args.Latitude) > 90 || math.Abs(*args.Longitude) > 180 || *args.Radius < 1 || *args.Radius > 100000 {
		return ErrInvalid
	}
	return nil
}

func init() {
	Collections["iphone.location"] = "ipl"
}

func IPhoneLocationCommand(op string) bool {
	return op == "iphone.location.current" || op == "iphone.location.significant" || op == "iphone.geofence.add" || op == "iphone.geofence.remove" || op == "iphone.geofence.list"
}

func ValidateIPhoneLocationSummary(raw []byte) error {
	var summary struct {
		Count       *int  `json:"count"`
		Significant *bool `json:"significant"`
	}
	if DecodeInput(raw, &summary) != nil || summary.Count == nil || *summary.Count < 0 || *summary.Count > 20 || summary.Significant == nil {
		return ErrInvalid
	}
	return nil
}
