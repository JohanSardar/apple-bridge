package demo

import (
	"time"
)

var Epoch = time.Date(2026, 10, 20, 16, 0, 0, 0, time.UTC)

var Effects = []string{
	"com.apple.MobileSMS.expressivesend.impact",
	"com.apple.MobileSMS.expressivesend.loud",
	"com.apple.MobileSMS.expressivesend.gentle",
	"com.apple.MobileSMS.expressivesend.invisibleink",
	"com.apple.messages.effect.CKEchoEffect",
	"com.apple.messages.effect.CKSpotlightEffect",
	"com.apple.messages.effect.CKHappyBirthdayEffect",
	"com.apple.messages.effect.CKConfettiEffect",
	"com.apple.messages.effect.CKHeartEffect",
	"com.apple.messages.effect.CKLasersEffect",
	"com.apple.messages.effect.CKFireworksEffect",
	"com.apple.messages.effect.CKSparklesEffect",
}
