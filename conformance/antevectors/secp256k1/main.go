// Command secp256k1 writes edge-case vectors for cosmos-sdk's secp256k1
// `PubKey.VerifySignature` (celestia-app v10.4.0: celestiaorg/cosmos-sdk
// v0.52.12, decred secp256k1 v4, no cgo). Every expected result is what
// that Go function returns, so the Rust verifier is checked against Go
// rather than against the cases' intent.
//
//	cd conformance/antevectors
//	GOPATH=/tmp/claude-0/gopath GOFLAGS=-mod=mod go run ./secp256k1 ../fixtures/secp256k1/verify.json
//	cargo test -p x-ante --test secp256k1_vectors
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"os"

	cosmossecp "github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1"
	"github.com/decred/dcrd/dcrec/secp256k1/v4"
	"github.com/decred/dcrd/dcrec/secp256k1/v4/ecdsa"
)

type vector struct {
	Name   string `json:"name"`
	PubKey string `json:"pub_key"`
	Msg    string `json:"msg"`
	Sig    string `json:"sig"`
	Valid  bool   `json:"valid"`
}

var (
	curveN = secp256k1.S256().N
	curveP = secp256k1.S256().P
	out    []vector
)

func add(name string, pub, msg, sig []byte) {
	ok := (&cosmossecp.PubKey{Key: pub}).VerifySignature(msg, sig)
	out = append(out, vector{name, hex.EncodeToString(pub), hex.EncodeToString(msg), hex.EncodeToString(sig), ok})
}

// be32 is the 32-byte big-endian encoding of v (which must fit).
func be32(v *big.Int) []byte {
	b := make([]byte, 32)
	v.FillBytes(b)
	return b
}

func cat(parts ...[]byte) []byte {
	var b []byte
	for _, p := range parts {
		b = append(b, p...)
	}
	return b
}

// deterministic private keys and messages.
func privKey(i int) *secp256k1.PrivateKey {
	h := sha256.Sum256([]byte(fmt.Sprintf("secp256k1-vector-key-%d", i)))
	return secp256k1.PrivKeyFromBytes(h[:])
}

func message(i int) []byte {
	return []byte(fmt.Sprintf("secp256k1 vector message %d", i))
}

// sign returns the 64-byte r || s low-S signature of sha256(msg).
func sign(k *secp256k1.PrivateKey, msg []byte) []byte {
	h := sha256.Sum256(msg)
	sig := ecdsa.SignCompact(k, h[:], true) // [recovery byte] r s, low S
	return sig[1:]
}

func hybrid(pub *secp256k1.PublicKey, rightParity bool) []byte {
	b := pub.SerializeUncompressed()
	odd := b[64]&1 == 1
	if odd == rightParity {
		b[0] = 0x07
	} else {
		b[0] = 0x06
	}
	return b
}

// crafted returns a public key for which (r, s) is a valid signature of
// msg, where r = R.x mod n for the curve point R with x coordinate x:
// Q = r^-1 (s R - e G). It lets the vectors use chosen r and s values that
// no honest signer produces (tiny s, R.x >= n, ...).
func crafted(x *big.Int, s *big.Int, msg []byte) (pub []byte, rBytes []byte, ok bool) {
	var fx, fy secp256k1.FieldVal
	if overflow := fx.SetByteSlice(be32(x)); overflow {
		return nil, nil, false
	}
	if !secp256k1.DecompressY(&fx, false, &fy) {
		return nil, nil, false
	}
	var one secp256k1.FieldVal
	one.SetInt(1)
	R := secp256k1.MakeJacobianPoint(&fx, &fy, &one)
	var r, sc, e secp256k1.ModNScalar
	r.SetByteSlice(be32(x))
	sc.SetByteSlice(be32(s))
	h := sha256.Sum256(msg)
	e.SetByteSlice(h[:])
	if r.IsZero() {
		return nil, nil, false
	}
	var sR, eG, sum, Q secp256k1.JacobianPoint
	secp256k1.ScalarMultNonConst(&sc, &R, &sR)
	e.Negate()
	secp256k1.ScalarBaseMultNonConst(&e, &eG)
	secp256k1.AddNonConst(&sR, &eG, &sum)
	rInv := new(secp256k1.ModNScalar).Set(&r).InverseNonConst()
	secp256k1.ScalarMultNonConst(rInv, &sum, &Q)
	Q.ToAffine()
	rb := r.Bytes()
	return secp256k1.NewPublicKey(&Q.X, &Q.Y).SerializeCompressed(), rb[:], true
}

// curveX returns the smallest x >= from that is the x coordinate of a curve point.
func curveX(from *big.Int) *big.Int {
	x := new(big.Int).Set(from)
	for {
		var fx, fy secp256k1.FieldVal
		if x.Cmp(curveP) < 0 && !fx.SetByteSlice(be32(x)) && secp256k1.DecompressY(&fx, false, &fy) {
			return x
		}
		x.Add(x, big.NewInt(1))
	}
}

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: secp256k1 <out.json>")
		os.Exit(2)
	}
	halfN := new(big.Int).Rsh(curveN, 1)

	// Honest signatures in every accepted key encoding.
	for i := 0; i < 16; i++ {
		k := privKey(i)
		pub := k.PubKey()
		msg := message(i)
		sig := sign(k, msg)
		r, s := new(big.Int).SetBytes(sig[:32]), new(big.Int).SetBytes(sig[32:])
		add(fmt.Sprintf("valid/compressed/%d", i), pub.SerializeCompressed(), msg, sig)
		add(fmt.Sprintf("valid/uncompressed/%d", i), pub.SerializeUncompressed(), msg, sig)
		add(fmt.Sprintf("valid/hybrid/%d", i), hybrid(pub, true), msg, sig)
		add(fmt.Sprintf("hybrid-wrong-parity/%d", i), hybrid(pub, false), msg, sig)
		add(fmt.Sprintf("wrong-message/%d", i), pub.SerializeCompressed(), message(i+1000), sig)
		add(fmt.Sprintf("wrong-key/%d", i), privKey(i+1000).PubKey().SerializeCompressed(), msg, sig)
		highS := new(big.Int).Sub(curveN, s)
		add(fmt.Sprintf("high-s/%d", i), pub.SerializeCompressed(), msg, cat(sig[:32], be32(highS)))
		add(fmt.Sprintf("r-zero/%d", i), pub.SerializeCompressed(), msg, cat(make([]byte, 32), sig[32:]))
		add(fmt.Sprintf("s-zero/%d", i), pub.SerializeCompressed(), msg, cat(sig[:32], make([]byte, 32)))
		add(fmt.Sprintf("r-n/%d", i), pub.SerializeCompressed(), msg, cat(be32(curveN), sig[32:]))
		add(fmt.Sprintf("s-n/%d", i), pub.SerializeCompressed(), msg, cat(sig[:32], be32(curveN)))
		if rn := new(big.Int).Add(r, curveN); rn.BitLen() <= 256 {
			add(fmt.Sprintf("r-plus-n/%d", i), pub.SerializeCompressed(), msg, cat(be32(rn), sig[32:]))
		}
		add(fmt.Sprintf("r-ff/%d", i), pub.SerializeCompressed(), msg, cat(be32(new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 256), big.NewInt(1))), sig[32:]))
		add(fmt.Sprintf("short-sig/%d", i), pub.SerializeCompressed(), msg, sig[:63])
		add(fmt.Sprintf("long-sig/%d", i), pub.SerializeCompressed(), msg, append(append([]byte{}, sig...), 0))
		add(fmt.Sprintf("der-sig/%d", i), pub.SerializeCompressed(), msg, ecdsa.Sign(k, func() []byte { h := sha256.Sum256(msg); return h[:] }()).Serialize())
		// Malformed keys.
		c := pub.SerializeCompressed()
		u := pub.SerializeUncompressed()
		for _, prefix := range []byte{0x00, 0x01, 0x04, 0x05, 0x06, 0x07, 0xff} {
			b := append([]byte{prefix}, c[1:]...)
			add(fmt.Sprintf("key33-prefix-%02x/%d", prefix, i), b, msg, sig)
		}
		for _, prefix := range []byte{0x00, 0x02, 0x03, 0x05, 0x08} {
			b := append([]byte{prefix}, u[1:]...)
			add(fmt.Sprintf("key65-prefix-%02x/%d", prefix, i), b, msg, sig)
		}
		add(fmt.Sprintf("key-len-32/%d", i), c[1:], msg, sig)
		add(fmt.Sprintf("key-len-34/%d", i), append(append([]byte{}, c...), 0), msg, sig)
		add(fmt.Sprintf("key-len-64/%d", i), u[1:], msg, sig)
		add(fmt.Sprintf("key-len-66/%d", i), append(append([]byte{}, u...), 0), msg, sig)
		add(fmt.Sprintf("key-empty/%d", i), nil, msg, sig)
		notOnCurve := append([]byte{}, u...)
		notOnCurve[64] ^= 1
		add(fmt.Sprintf("key65-not-on-curve/%d", i), notOnCurve, msg, sig)
	}

	// Keys whose coordinates are >= p or not on the curve.
	k := privKey(0)
	msg := message(0)
	sig := sign(k, msg)
	pBytes := be32(curveP)
	add("key33-x-eq-p", append([]byte{0x02}, pBytes...), msg, sig)
	add("key33-x-ff", append([]byte{0x03}, be32(new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 256), big.NewInt(1)))...), msg, sig)
	u := k.PubKey().SerializeUncompressed()
	yPlusP := new(big.Int).Add(new(big.Int).SetBytes(u[33:]), curveP)
	if yPlusP.BitLen() <= 256 {
		add("key65-y-plus-p", cat(u[:33], be32(yPlusP)), msg, sig)
	}
	add("key65-x-eq-p", cat([]byte{0x04}, pBytes, u[33:]), msg, sig)
	for x := int64(0); x < 8; x++ {
		var fx, fy secp256k1.FieldVal
		fx.SetInt(uint16(x))
		if !secp256k1.DecompressY(&fx, false, &fy) {
			add(fmt.Sprintf("key33-x-%d-not-on-curve", x), append([]byte{0x02}, be32(big.NewInt(x))...), msg, sig)
		}
	}

	// Crafted keys: chosen r and s.
	for i, s := range []*big.Int{big.NewInt(1), big.NewInt(2), big.NewInt(12345), halfN, new(big.Int).Add(halfN, big.NewInt(1)), new(big.Int).Sub(curveN, big.NewInt(1))} {
		x := curveX(big.NewInt(int64(1000 + i)))
		m := message(2000 + i)
		pub, rb, ok := crafted(x, s, m)
		if !ok {
			panic("crafted")
		}
		name := fmt.Sprintf("crafted/s-%d", i)
		add(name, pub, m, cat(rb, be32(s)))
		// s + n still fits in 32 bytes for tiny s: Go reduces it mod n.
		if sn := new(big.Int).Add(s, curveN); sn.BitLen() <= 256 {
			add(name+"/s-plus-n", pub, m, cat(rb, be32(sn)))
		}
		// r = R.x is tiny, so r + n fits too.
		rn := new(big.Int).Add(new(big.Int).SetBytes(rb), curveN)
		if rn.BitLen() <= 256 {
			add(name+"/r-plus-n", pub, m, cat(be32(rn), be32(s)))
		}
	}
	// R.x >= n: the signature carries r = R.x - n (Go's second comparison).
	for i := 0; i < 4; i++ {
		x := curveX(new(big.Int).Add(curveN, big.NewInt(int64(1+i*7919)))) // R.x = n would make r zero
		m := message(3000 + i)
		s := big.NewInt(int64(7 + i))
		pub, rb, ok := crafted(x, s, m)
		if !ok {
			panic("crafted r >= n")
		}
		add(fmt.Sprintf("crafted/rx-ge-n/%d", i), pub, m, cat(rb, be32(s)))
		add(fmt.Sprintf("crafted/rx-ge-n/%d/r-plus-n", i), pub, m, cat(be32(new(big.Int).Add(new(big.Int).SetBytes(rb), curveN)), be32(s)))
		add(fmt.Sprintf("crafted/rx-ge-n/%d/wrong-msg", i), pub, message(4000+i), cat(rb, be32(s)))
	}
	// Crafted with a hybrid / uncompressed encoding of the crafted key.
	{
		x := curveX(big.NewInt(77))
		m := message(5000)
		pub, rb, _ := crafted(x, big.NewInt(3), m)
		pk, err := secp256k1.ParsePubKey(pub)
		if err != nil {
			panic(err)
		}
		add("crafted/uncompressed", pk.SerializeUncompressed(), m, cat(rb, be32(big.NewInt(3))))
		add("crafted/hybrid", hybrid(pk, true), m, cat(rb, be32(big.NewInt(3))))
	}

	valid := 0
	for _, v := range out {
		if v.Valid {
			valid++
		}
	}
	b, err := json.MarshalIndent(out, "", " ")
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(os.Args[1], append(b, '\n'), 0o644); err != nil {
		panic(err)
	}
	fmt.Printf("%d vectors (%d valid)\n", len(out), valid)
}
