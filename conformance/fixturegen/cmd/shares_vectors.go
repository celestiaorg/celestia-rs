package cmd

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"flag"
	"fmt"
	"sort"

	sqblob "github.com/celestiaorg/go-square/blob"
	sqnamespace "github.com/celestiaorg/go-square/namespace"
	sqshares "github.com/celestiaorg/go-square/shares"
)

func init() {
	All = append(All,
		Subcommand{
			Name:        "shares-namespaces",
			Description: "Emit Celestia share namespace vectors.",
			Run:         runSharesNamespaces,
		},
		Subcommand{
			Name:        "shares-info-reserved",
			Description: "Emit Celestia share info-byte and reserved-byte vectors.",
			Run:         runSharesInfoReserved,
		},
		Subcommand{
			Name:        "shares-builders",
			Description: "Emit Celestia share builder vectors.",
			Run:         runSharesBuilders,
		},
		Subcommand{
			Name:        "shares-compact",
			Description: "Emit Celestia compact transaction share vectors.",
			Run:         runSharesCompact,
		},
		Subcommand{
			Name:        "shares-sparse",
			Description: "Emit Celestia sparse blob share vectors.",
			Run:         runSharesSparse,
		},
		Subcommand{
			Name:        "shares-padding",
			Description: "Emit Celestia share padding vectors.",
			Run:         runSharesPadding,
		},
		Subcommand{
			Name:        "shares-malformed",
			Description: "Emit malformed Celestia share cases.",
			Run:         runSharesMalformed,
		},
	)
}

type sharesNamespacesFixture struct {
	Scenario                   string                    `json:"scenario"`
	NamespaceSize              int                       `json:"namespace_size"`
	NamespaceIDSize            int                       `json:"namespace_id_size"`
	NamespaceVersionZeroIDSize int                       `json:"namespace_version_zero_id_size"`
	NamespaceVersionZeroPrefix int                       `json:"namespace_version_zero_prefix_size"`
	Constants                  []sharesNamespaceCase     `json:"constants"`
	Constructors               []sharesNamespaceCase     `json:"constructors"`
	OrderingHex                []string                  `json:"ordering_hex"`
	Malformed                  []sharesMalformedMiniCase `json:"malformed"`
}

type sharesNamespaceCase struct {
	Label        string `json:"label"`
	NamespaceHex string `json:"namespace_hex"`
	Version      uint8  `json:"version"`
	IDHex        string `json:"id_hex"`
}

type sharesMalformedMiniCase struct {
	Label         string `json:"label"`
	ExpectedError string `json:"expected_error"`
}

type sharesInfoReservedFixture struct {
	Scenario      string                    `json:"scenario"`
	InfoCases     []sharesInfoCase          `json:"info_cases"`
	ReservedCases []sharesReservedCase      `json:"reserved_cases"`
	Malformed     []sharesMalformedMiniCase `json:"malformed"`
}

type sharesInfoCase struct {
	Label           string `json:"label"`
	Version         uint8  `json:"version"`
	IsSequenceStart bool   `json:"is_sequence_start"`
	ByteHex         string `json:"byte_hex"`
}

type sharesReservedCase struct {
	Label     string `json:"label"`
	ByteIndex uint32 `json:"byte_index"`
	BytesHex  string `json:"bytes_hex"`
}

type sharesBuildersFixture struct {
	Scenario string              `json:"scenario"`
	Cases    []sharesBuilderCase `json:"cases"`
}

type sharesBuilderCase struct {
	Label                string `json:"label"`
	NamespaceHex         string `json:"namespace_hex"`
	ShareVersion         uint8  `json:"share_version"`
	IsFirstShare         bool   `json:"is_first_share"`
	InitialAvailable     int    `json:"initial_available"`
	PaddingBytes         int    `json:"padding_bytes"`
	ShareHex             string `json:"share_hex"`
	RawDataHex           string `json:"raw_data_hex"`
	RawDataReservedHex   string `json:"raw_data_reserved_hex,omitempty"`
	ReservedByteIndex    uint32 `json:"reserved_byte_index,omitempty"`
	SequenceLen          uint32 `json:"sequence_len,omitempty"`
	IsCompact            bool   `json:"is_compact"`
	IsEmptyBeforeData    bool   `json:"is_empty_before_data"`
	IsSequenceStartAfter bool   `json:"is_sequence_start_after"`
}

type sharesCompactFixture struct {
	Scenario       string                    `json:"scenario"`
	DelimitedCases []sharesDelimitedTxCase   `json:"delimited_cases"`
	SplitCases     []sharesCompactSplitCase  `json:"split_cases"`
	IndexWrapper   sharesIndexWrapperFixture `json:"index_wrapper"`
}

type sharesDelimitedTxCase struct {
	Label  string `json:"label"`
	TxHex  string `json:"tx_hex"`
	OutHex string `json:"out_hex"`
}

type sharesCompactSplitCase struct {
	Label        string                 `json:"label"`
	InputTxsHex  []string               `json:"input_txs_hex"`
	TxSharesHex  []string               `json:"tx_shares_hex"`
	PFBSharesHex []string               `json:"pfb_shares_hex"`
	ParsedTxsHex []string               `json:"parsed_txs_hex"`
	ParsedPFBHex []string               `json:"parsed_pfb_hex"`
	ShareRanges  []sharesShareRangeCase `json:"share_ranges"`
}

type sharesShareRangeCase struct {
	TxHashHex string `json:"tx_hash_hex"`
	Start     int    `json:"start"`
	End       int    `json:"end"`
}

type sharesIndexWrapperFixture struct {
	TxHex         string   `json:"tx_hex"`
	ShareIndexes  []uint32 `json:"share_indexes"`
	EncodedHex    string   `json:"encoded_hex"`
	NonWrapperHex string   `json:"non_wrapper_hex"`
}

type sharesSparseFixture struct {
	Scenario string             `json:"scenario"`
	Cases    []sharesSparseCase `json:"cases"`
}

type sharesSparseCase struct {
	Label        string                 `json:"label"`
	NamespaceHex string                 `json:"namespace_hex"`
	DataHex      string                 `json:"data_hex"`
	ShareVersion uint8                  `json:"share_version"`
	SharesHex    []string               `json:"shares_hex"`
	ParsedBlobs  []sharesParsedBlobCase `json:"parsed_blobs"`
}

type sharesParsedBlobCase struct {
	NamespaceHex string `json:"namespace_hex"`
	DataHex      string `json:"data_hex"`
	ShareVersion uint8  `json:"share_version"`
}

type sharesPaddingFixture struct {
	Scenario                 string   `json:"scenario"`
	NamespacePaddingShareHex string   `json:"namespace_padding_share_hex"`
	ReservedPaddingShareHex  string   `json:"reserved_padding_share_hex"`
	TailPaddingShareHex      string   `json:"tail_padding_share_hex"`
	NamespacePaddingCountHex []string `json:"namespace_padding_count_hex"`
	ReservedPaddingCountHex  []string `json:"reserved_padding_count_hex"`
	TailPaddingCountHex      []string `json:"tail_padding_count_hex"`
}

type sharesMalformedFixture struct {
	Scenario string                `json:"scenario"`
	Cases    []sharesMalformedCase `json:"cases"`
}

type sharesMalformedCase struct {
	Label             string `json:"label"`
	Operation         string `json:"operation"`
	InputHex          string `json:"input_hex,omitempty"`
	ExpectedError     string `json:"expected_error"`
	SupportedVersions []int  `json:"supported_versions,omitempty"`
}

func runSharesNamespaces(args []string) error {
	fs := flag.NewFlagSet("shares-namespaces", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/shares/namespaces.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	constructors := []sharesNamespaceCase{
		namespaceCase("new-v0-empty", mustNamespace(sqnamespace.NewV0(nil))),
		namespaceCase("new-v0-short-sub-id", mustNamespace(sqnamespace.NewV0([]byte{0xab, 0xcd}))),
		namespaceCase("new-v0-full-sub-id", mustNamespace(sqnamespace.NewV0([]byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10}))),
		namespaceCase("new-max-version", mustNamespace(sqnamespace.New(sqnamespace.NamespaceVersionMax, bytes.Repeat([]byte{0xff}, sqnamespace.NamespaceIDSize)))),
	}
	ordering := []string{
		hexNamespace(constructors[0]),
		hexNamespace(namespaceCase("tx", sqnamespace.TxNamespace)),
		hexNamespace(constructors[1]),
		hexNamespace(namespaceCase("tail", sqnamespace.TailPaddingNamespace)),
		hexNamespace(namespaceCase("parity", sqnamespace.ParitySharesNamespace)),
	}
	sort.Strings(ordering)

	fixture := sharesNamespacesFixture{
		Scenario:                   "shares-namespaces",
		NamespaceSize:              sqnamespace.NamespaceSize,
		NamespaceIDSize:            sqnamespace.NamespaceIDSize,
		NamespaceVersionZeroIDSize: sqnamespace.NamespaceVersionZeroIDSize,
		NamespaceVersionZeroPrefix: sqnamespace.NamespaceVersionZeroPrefixSize,
		Constants: []sharesNamespaceCase{
			namespaceCase("tx", sqnamespace.TxNamespace),
			namespaceCase("intermediate-state-roots", sqnamespace.IntermediateStateRootsNamespace),
			namespaceCase("pay-for-blob", sqnamespace.PayForBlobNamespace),
			namespaceCase("primary-reserved-padding", sqnamespace.PrimaryReservedPaddingNamespace),
			namespaceCase("max-primary-reserved", sqnamespace.MaxPrimaryReservedNamespace),
			namespaceCase("min-secondary-reserved", sqnamespace.MinSecondaryReservedNamespace),
			namespaceCase("tail-padding", sqnamespace.TailPaddingNamespace),
			namespaceCase("parity-shares", sqnamespace.ParitySharesNamespace),
		},
		Constructors: constructors,
		OrderingHex:  ordering,
		Malformed: []sharesMalformedMiniCase{
			{Label: "short-namespace", ExpectedError: namespaceErrorName(func() error { _, err := sqnamespace.From([]byte{0}); return err })},
			{Label: "unsupported-version", ExpectedError: namespaceErrorName(func() error { _, err := sqnamespace.New(1, make([]byte, sqnamespace.NamespaceIDSize)); return err })},
			{Label: "short-id", ExpectedError: namespaceErrorName(func() error { _, err := sqnamespace.New(0, []byte{1}); return err })},
			{Label: "invalid-v0-prefix", ExpectedError: namespaceErrorName(func() error {
				id := make([]byte, sqnamespace.NamespaceIDSize)
				id[0] = 1
				_, err := sqnamespace.New(0, id)
				return err
			})},
			{Label: "long-v0-sub-id", ExpectedError: namespaceErrorName(func() error {
				_, err := sqnamespace.NewV0(bytes.Repeat([]byte{1}, sqnamespace.NamespaceVersionZeroIDSize+1))
				return err
			})},
		},
	}
	return writeSharesJSON(*out, fixture)
}

func runSharesInfoReserved(args []string) error {
	fs := flag.NewFlagSet("shares-info-reserved", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/shares/info-reserved.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	var infoCases []sharesInfoCase
	for _, version := range []uint8{0, 1, sqshares.MaxShareVersion} {
		for _, start := range []bool{false, true} {
			info, err := sqshares.NewInfoByte(version, start)
			if err != nil {
				return err
			}
			infoCases = append(infoCases, sharesInfoCase{
				Label:           fmt.Sprintf("version-%d-start-%t", version, start),
				Version:         version,
				IsSequenceStart: start,
				ByteHex:         hex.EncodeToString([]byte{byte(info)}),
			})
		}
	}

	var reservedCases []sharesReservedCase
	for _, index := range []uint32{0, 34, 38, sqshares.ShareSize - 1} {
		reserved, err := sqshares.NewReservedBytes(index)
		if err != nil {
			return err
		}
		reservedCases = append(reservedCases, sharesReservedCase{
			Label:     fmt.Sprintf("index-%d", index),
			ByteIndex: index,
			BytesHex:  hex.EncodeToString(reserved),
		})
	}

	fixture := sharesInfoReservedFixture{
		Scenario:      "shares-info-reserved",
		InfoCases:     infoCases,
		ReservedCases: reservedCases,
		Malformed: []sharesMalformedMiniCase{
			{Label: "info-version-too-large", ExpectedError: shareErrorName(func() error { _, err := sqshares.NewInfoByte(sqshares.MaxShareVersion+1, false); return err })},
			{Label: "reserved-short", ExpectedError: shareErrorName(func() error { _, err := sqshares.ParseReservedBytes([]byte{0, 1, 2}); return err })},
			{Label: "reserved-out-of-range", ExpectedError: shareErrorName(func() error { _, err := sqshares.NewReservedBytes(sqshares.ShareSize); return err })},
		},
	}
	return writeSharesJSON(*out, fixture)
}

func runSharesBuilders(args []string) error {
	fs := flag.NewFlagSet("shares-builders", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/shares/builders.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	ns := mustNamespace(sqnamespace.NewV0([]byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10}))
	cases := []sharesBuilderCase{
		mustBuilderCase("compact-first", sqnamespace.TxNamespace, 0, true, []byte("abc")),
		mustBuilderCase("compact-continuation", sqnamespace.TxNamespace, 0, false, bytes.Repeat([]byte{0x11}, 12)),
		mustBuilderCase("sparse-first", ns, 0, true, []byte("blob-data")),
		mustBuilderCase("sparse-continuation", ns, 0, false, bytes.Repeat([]byte{0x22}, 9)),
	}
	fixture := sharesBuildersFixture{Scenario: "shares-builders", Cases: cases}
	return writeSharesJSON(*out, fixture)
}

func runSharesCompact(args []string) error {
	fs := flag.NewFlagSet("shares-compact", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/shares/compact.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	wrappedTx, err := sqblob.MarshalIndexWrapper([]byte("pfb"), 4, 9)
	if err != nil {
		return err
	}
	splitCases, err := compactSplitCases([]compactSplitInput{
		{label: "single-small", txs: [][]byte{[]byte("alpha")}},
		{label: "spans-shares", txs: [][]byte{bytes.Repeat([]byte{0x07}, sqshares.FirstCompactShareContentSize+10)}},
		{label: "ordinary-and-pfb", txs: [][]byte{[]byte("ordinary"), wrappedTx}},
	})
	if err != nil {
		return err
	}

	fixture := sharesCompactFixture{
		Scenario: "shares-compact",
		DelimitedCases: []sharesDelimitedTxCase{
			delimitedCase("empty", nil),
			delimitedCase("abc", []byte("abc")),
			delimitedCase("long", bytes.Repeat([]byte{0x09}, 130)),
		},
		SplitCases: splitCases,
		IndexWrapper: sharesIndexWrapperFixture{
			TxHex:         hex.EncodeToString([]byte("pfb")),
			ShareIndexes:  []uint32{4, 9},
			EncodedHex:    hex.EncodeToString(wrappedTx),
			NonWrapperHex: hex.EncodeToString([]byte("not protobuf")),
		},
	}
	return writeSharesJSON(*out, fixture)
}

func runSharesSparse(args []string) error {
	fs := flag.NewFlagSet("shares-sparse", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/shares/sparse.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	nsA := mustNamespace(sqnamespace.NewV0([]byte{0xa, 0xb, 0xc}))
	nsB := mustNamespace(sqnamespace.NewV0([]byte{1, 2, 3, 4, 5, 6, 7, 8, 9, 10}))
	cases, err := sparseCases([]sparseInput{
		{label: "single-share", ns: nsA, data: []byte("blob-data")},
		{label: "spans-shares", ns: nsB, data: bytes.Repeat([]byte{0x33}, sqshares.FirstSparseShareContentSize+5)},
	})
	if err != nil {
		return err
	}
	fixture := sharesSparseFixture{Scenario: "shares-sparse", Cases: cases}
	return writeSharesJSON(*out, fixture)
}

func runSharesPadding(args []string) error {
	fs := flag.NewFlagSet("shares-padding", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/shares/padding.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	ns := mustNamespace(sqnamespace.NewV0([]byte{0xa, 0xb, 0xc}))
	namespacePadding, err := sqshares.NamespacePaddingShare(ns, sqshares.ShareVersionZero)
	if err != nil {
		return err
	}
	namespacePaddingMany, err := sqshares.NamespacePaddingShares(ns, sqshares.ShareVersionZero, 2)
	if err != nil {
		return err
	}
	fixture := sharesPaddingFixture{
		Scenario:                 "shares-padding",
		NamespacePaddingShareHex: hexShare(namespacePadding),
		ReservedPaddingShareHex:  hexShare(sqshares.ReservedPaddingShare()),
		TailPaddingShareHex:      hexShare(sqshares.TailPaddingShare()),
		NamespacePaddingCountHex: hexShares(namespacePaddingMany),
		ReservedPaddingCountHex:  hexShares(sqshares.ReservedPaddingShares(2)),
		TailPaddingCountHex:      hexShares(sqshares.TailPaddingShares(2)),
	}
	return writeSharesJSON(*out, fixture)
}

func runSharesMalformed(args []string) error {
	fs := flag.NewFlagSet("shares-malformed", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/shares/malformed.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	orphan := mustContinuationShare()
	unsupported := mustUnsupportedSparseShare()
	badReserved := mustBadReservedCompactShare()
	fixture := sharesMalformedFixture{
		Scenario: "shares-malformed",
		Cases: []sharesMalformedCase{
			{Label: "short-share", Operation: "share_from_slice", InputHex: "00", ExpectedError: "invalid_share_length"},
			{Label: "orphan-sparse-continuation", Operation: "parse_sparse", InputHex: hexShare(orphan), ExpectedError: "invalid_sparse_share"},
			{Label: "unsupported-sparse-version", Operation: "parse_sparse", InputHex: hexShare(unsupported), SupportedVersions: []int{0}, ExpectedError: "unsupported_share_version"},
			{Label: "compact-reserved-out-of-range", Operation: "parse_compact", InputHex: hexShare(badReserved), SupportedVersions: []int{0}, ExpectedError: "invalid_reserved_byte_index"},
		},
	}
	return writeSharesJSON(*out, fixture)
}

func writeSharesJSON(path string, v any) error {
	if err := writeJSON(path, v); err != nil {
		return err
	}
	fmt.Printf("wrote %s\n", path)
	return nil
}

func namespaceCase(label string, ns sqnamespace.Namespace) sharesNamespaceCase {
	return sharesNamespaceCase{
		Label:        label,
		NamespaceHex: hex.EncodeToString(ns.Bytes()),
		Version:      ns.Version,
		IDHex:        hex.EncodeToString(ns.ID),
	}
}

func hexNamespace(c sharesNamespaceCase) string {
	return c.NamespaceHex
}

func mustNamespace(ns sqnamespace.Namespace, err error) sqnamespace.Namespace {
	if err != nil {
		panic(err)
	}
	return ns
}

func namespaceErrorName(fn func() error) string {
	if err := fn(); err == nil {
		return ""
	}
	return "invalid_namespace"
}

func shareErrorName(fn func() error) string {
	if err := fn(); err == nil {
		return ""
	}
	return "invalid_share"
}

func mustBuilderCase(label string, ns sqnamespace.Namespace, version uint8, first bool, data []byte) sharesBuilderCase {
	builder, err := sqshares.NewBuilder(ns, version, first)
	if err != nil {
		panic(err)
	}
	initialAvailable := builder.AvailableBytes()
	isEmpty := builder.IsEmptyShare()
	isCompact := ns.IsTx() || ns.IsPayForBlob()
	var reservedIndex uint32
	if isCompact {
		if err := builder.MaybeWriteReservedBytes(); err != nil {
			panic(err)
		}
		reservedIndex = uint32(sqshares.ShareSize - builder.AvailableBytes())
	}
	if first {
		if err := builder.WriteSequenceLen(uint32(len(data))); err != nil {
			panic(err)
		}
	}
	if left := builder.AddData(data); left != nil {
		panic("builder test data unexpectedly overflowed")
	}
	padding := builder.ZeroPadIfNecessary()
	share, err := builder.Build()
	if err != nil {
		panic(err)
	}
	rawData, err := share.RawData()
	if err != nil {
		panic(err)
	}
	info, err := share.InfoByte()
	if err != nil {
		panic(err)
	}
	out := sharesBuilderCase{
		Label:                label,
		NamespaceHex:         hex.EncodeToString(ns.Bytes()),
		ShareVersion:         version,
		IsFirstShare:         first,
		InitialAvailable:     initialAvailable,
		PaddingBytes:         padding,
		ShareHex:             hexShare(*share),
		RawDataHex:           hex.EncodeToString(rawData),
		ReservedByteIndex:    reservedIndex,
		SequenceLen:          uint32(len(data)),
		IsCompact:            isCompact,
		IsEmptyBeforeData:    isEmpty,
		IsSequenceStartAfter: info.IsSequenceStart(),
	}
	if isCompact {
		rawReserved, err := share.RawDataUsingReserved()
		if err != nil {
			panic(err)
		}
		out.RawDataReservedHex = hex.EncodeToString(rawReserved)
	}
	return out
}

type compactSplitInput struct {
	label string
	txs   [][]byte
}

func compactSplitCases(inputs []compactSplitInput) ([]sharesCompactSplitCase, error) {
	cases := make([]sharesCompactSplitCase, 0, len(inputs))
	for _, input := range inputs {
		txShares, pfbShares, ranges, err := sqshares.SplitTxs(input.txs)
		if err != nil {
			return nil, err
		}
		parsedTxs, err := sqshares.ParseTxs(txShares)
		if err != nil {
			return nil, err
		}
		parsedPFB, err := sqshares.ParseTxs(pfbShares)
		if err != nil {
			return nil, err
		}
		cases = append(cases, sharesCompactSplitCase{
			Label:        input.label,
			InputTxsHex:  hexByteSlices(input.txs),
			TxSharesHex:  hexShares(txShares),
			PFBSharesHex: hexShares(pfbShares),
			ParsedTxsHex: hexByteSlices(parsedTxs),
			ParsedPFBHex: hexByteSlices(parsedPFB),
			ShareRanges:  shareRangeCases(ranges),
		})
	}
	return cases, nil
}

func delimitedCase(label string, tx []byte) sharesDelimitedTxCase {
	out, err := sqshares.MarshalDelimitedTx(tx)
	if err != nil {
		panic(err)
	}
	return sharesDelimitedTxCase{
		Label:  label,
		TxHex:  hex.EncodeToString(tx),
		OutHex: hex.EncodeToString(out),
	}
}

func shareRangeCases(ranges map[[sha256.Size]byte]sqshares.Range) []sharesShareRangeCase {
	cases := make([]sharesShareRangeCase, 0, len(ranges))
	for key, shareRange := range ranges {
		cases = append(cases, sharesShareRangeCase{
			TxHashHex: hex.EncodeToString(key[:]),
			Start:     shareRange.Start,
			End:       shareRange.End,
		})
	}
	sort.Slice(cases, func(i, j int) bool { return cases[i].TxHashHex < cases[j].TxHashHex })
	return cases
}

type sparseInput struct {
	label string
	ns    sqnamespace.Namespace
	data  []byte
}

func sparseCases(inputs []sparseInput) ([]sharesSparseCase, error) {
	cases := make([]sharesSparseCase, 0, len(inputs))
	for _, input := range inputs {
		blob := sqblob.New(input.ns, input.data, sqshares.ShareVersionZero)
		shares, err := sqshares.SplitBlobs(blob)
		if err != nil {
			return nil, err
		}
		parsed, err := sqshares.ParseBlobs(shares)
		if err != nil {
			return nil, err
		}
		cases = append(cases, sharesSparseCase{
			Label:        input.label,
			NamespaceHex: hex.EncodeToString(input.ns.Bytes()),
			DataHex:      hex.EncodeToString(input.data),
			ShareVersion: sqshares.ShareVersionZero,
			SharesHex:    hexShares(shares),
			ParsedBlobs:  parsedBlobCases(parsed),
		})
	}
	return cases, nil
}

func parsedBlobCases(blobs []*sqblob.Blob) []sharesParsedBlobCase {
	cases := make([]sharesParsedBlobCase, 0, len(blobs))
	for _, blob := range blobs {
		cases = append(cases, sharesParsedBlobCase{
			NamespaceHex: hex.EncodeToString(blob.Namespace().Bytes()),
			DataHex:      hex.EncodeToString(blob.Data),
			ShareVersion: uint8(blob.ShareVersion),
		})
	}
	return cases
}

func mustContinuationShare() sqshares.Share {
	ns := mustNamespace(sqnamespace.NewV0([]byte{0xaa}))
	builder, err := sqshares.NewBuilder(ns, sqshares.ShareVersionZero, false)
	if err != nil {
		panic(err)
	}
	builder.AddData([]byte("orphan"))
	builder.ZeroPadIfNecessary()
	share, err := builder.Build()
	if err != nil {
		panic(err)
	}
	return *share
}

func mustUnsupportedSparseShare() sqshares.Share {
	ns := mustNamespace(sqnamespace.NewV0([]byte{0xbb}))
	builder, err := sqshares.NewBuilder(ns, 1, true)
	if err != nil {
		panic(err)
	}
	if err := builder.WriteSequenceLen(1); err != nil {
		panic(err)
	}
	builder.AddData([]byte{0x01})
	builder.ZeroPadIfNecessary()
	share, err := builder.Build()
	if err != nil {
		panic(err)
	}
	return *share
}

func mustBadReservedCompactShare() sqshares.Share {
	splitter := sqshares.NewCompactShareSplitter(sqnamespace.TxNamespace, sqshares.ShareVersionZero)
	if err := splitter.WriteTx([]byte("bad-reserved")); err != nil {
		panic(err)
	}
	shares, err := splitter.Export()
	if err != nil {
		panic(err)
	}
	raw := append([]byte{}, shares[0].ToBytes()...)
	binary.BigEndian.PutUint32(raw[sqnamespace.NamespaceSize+sqshares.ShareInfoBytes+sqshares.SequenceLenBytes:], sqshares.ShareSize)
	share, err := sqshares.NewShare(raw)
	if err != nil {
		panic(err)
	}
	return *share
}

func hexShare(share sqshares.Share) string {
	return hex.EncodeToString(share.ToBytes())
}

func hexShares(shares []sqshares.Share) []string {
	out := make([]string, 0, len(shares))
	for _, share := range shares {
		out = append(out, hexShare(share))
	}
	return out
}

func hexByteSlices(values [][]byte) []string {
	out := make([]string, 0, len(values))
	for _, value := range values {
		out = append(out, hex.EncodeToString(value))
	}
	return out
}
