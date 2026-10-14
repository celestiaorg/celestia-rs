package main

import "golang.org/x/crypto/curve25519"

func curve25519PublicKey(priv [32]byte) ([]byte, error) {
	return curve25519.X25519(priv[:], curve25519.Basepoint)
}
