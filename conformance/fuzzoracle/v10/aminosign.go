package main

import (
	"context"
	"encoding/hex"
	"fmt"
	"strings"

	signingv1beta1 "cosmossdk.io/api/cosmos/tx/signing/v1beta1"
	txsigning "cosmossdk.io/x/tx/signing"
	"github.com/cosmos/cosmos-sdk/client"
	sdk "github.com/cosmos/cosmos-sdk/types"
	authsigning "github.com/cosmos/cosmos-sdk/x/auth/signing"
)

// aminoSign: the SIGN_MODE_LEGACY_AMINO_JSON sign bytes of every signer
// of a tx, as the SigVerificationDecorator computes them (chain id "c",
// account number 7, the signer info's sequence; the legacy sign doc has no
// public key). Out is a
// comma-separated list: the hex of the sign bytes, or "err:" and the hex
// of the error text when the handler fails for that signer.
func aminoSign(txCfg client.TxConfig, dec sdk.TxDecoder, input []byte) (o out) {
	defer func() {
		if r := recover(); r != nil {
			o.Ok = false
			o.Panic = trunc(fmt.Sprint(r))
		}
	}()
	t, err := dec(input)
	if err != nil {
		o.Err = "decode: " + trunc(err.Error())
		return o
	}
	atx, ok := t.(authsigning.V2AdaptableTx)
	if !ok {
		o.Err = "not adaptable"
		return o
	}
	sigTx, ok := t.(authsigning.SigVerifiableTx)
	if !ok {
		o.Err = "not verifiable"
		return o
	}
	sigs, err := sigTx.GetSignaturesV2()
	if err != nil {
		o.Err = "signatures: " + trunc(err.Error())
		return o
	}
	signers, err := sigTx.GetSigners()
	if err != nil {
		o.Err = "signers: " + trunc(err.Error())
		return o
	}
	var outs []string
	for i, s := range sigs {
		sd := txsigning.SignerData{
			ChainID:       "c",
			AccountNumber: 7,
			Sequence:      s.Sequence,
		}
		if i < len(signers) {
			sd.Address = sdk.AccAddress(signers[i]).String()
		}
		bz, err := txCfg.SignModeHandler().GetSignBytes(context.Background(), signingv1beta1.SignMode_SIGN_MODE_LEGACY_AMINO_JSON, sd, atx.GetSigningTxData())
		if err != nil {
			outs = append(outs, "err:"+hex.EncodeToString([]byte(trunc(err.Error()))))
			continue
		}
		outs = append(outs, hex.EncodeToString(bz))
	}
	o.Ok = true
	o.Out = strings.Join(outs, ",")
	return o
}
