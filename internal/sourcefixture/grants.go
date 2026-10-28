package sourcefixture

import "github.com/JohanSardar/apple-bridge/internal/contracts"

func init() {
	contracts.RegisterSource("labdummy", "Lab dummy", true)
	contracts.Collections["labdummy.item"] = "ldm"
}
