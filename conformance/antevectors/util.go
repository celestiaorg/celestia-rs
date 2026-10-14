package main

import (
	"math/big"
	"sort"
)

var p256N, _ = new(big.Int).SetString("FFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551", 16)

func sortStrings(s []string) { sort.Strings(s) }

func toHighSN(sig []byte, n *big.Int) []byte {
	if len(sig) != 64 {
		return sig
	}
	s := new(big.Int).SetBytes(sig[32:])
	s.Sub(n, s)
	out := append([]byte{}, sig[:32]...)
	return append(out, s.FillBytes(make([]byte, 32))...)
}
