package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"cosmossdk.io/math"
	sdk "github.com/cosmos/cosmos-sdk/types"
)

func TestNewCoinsVectors(t *testing.T) {
	type coin struct {
		Denom  string `json:"denom"`
		Amount string `json:"amount"`
	}
	type row struct {
		Coins  []coin `json:"coins"`
		Result string `json:"result"`
		Panic  string `json:"panic"`
	}
	var rows []row
	for _, input := range [][]coin{
		{}, {{"utia", "1"}, {"utia", "2"}}, {{"utia", "-1"}},
		{{"!", "1"}}, {{"!", "0"}}, {{"!", "1"}, {"utia", ""}},
		{{"uzzz", "2"}, {"uabc", "1"}, {"utia", "0"}},
		{{"uzzz", "-1"}, {"uabc", "1"}},
	} {
		r := row{Coins: input}
		func() {
			defer func() {
				if value := recover(); value != nil {
					r.Panic = fmt.Sprint(value)
				}
			}()
			var coins []sdk.Coin
			for _, value := range input {
				var amount math.Int
				if value.Amount != "" {
					var ok bool
					amount, ok = math.NewIntFromString(value.Amount)
					if !ok {
						t.Fatal("invalid fixture amount")
					}
				}
				coins = append(coins, sdk.Coin{Denom: value.Denom, Amount: amount})
			}
			r.Result = sdk.NewCoins(coins...).String()
		}()
		rows = append(rows, r)
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	data = append(data, '\n')
	path := filepath.Join("..", "..", "fixtures", "new-coins.json")
	if os.Getenv("UPDATE_COIN_VECTORS") == "1" {
		if err := os.WriteFile(path, data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	old, err := os.ReadFile(path)
	if err != nil || string(old) != string(data) {
		t.Fatalf("NewCoins vectors differ: %v; got %s", err, data)
	}
}
