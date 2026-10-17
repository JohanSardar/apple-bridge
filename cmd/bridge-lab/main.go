package main

import (
	"fmt"
	"os"
)

func main() {
	fmt.Fprintln(os.Stderr, "scenario failed: process world is not implemented")
	os.Exit(1)
}
