package simapple

import (
	"bytes"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func simulatedAnnotatedPDF(annotations []contracts.PDFAnnotation) []byte {
	escape := func(s string) string {
		return strings.NewReplacer("\\", "\\\\", "(", "\\(", ")", "\\)", "\r", " ", "\n", " ").Replace(s)
	}
	refs := ""
	objects := []string{"<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "", "<< /Length 46 >>\nstream\nBT /F1 12 Tf 20 100 Td (Bridge fixture) Tj ET\nendstream", "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"}
	for i, a := range annotations {
		refs += fmt.Sprintf("%d 0 R ", i+6)
		kind := "Text"
		quad := ""
		if a.Type == "highlight" {
			kind = "Highlight"
			quad = fmt.Sprintf(" /QuadPoints [%g %g %g %g %g %g %g %g]", a.X, a.Y+a.Height, a.X+a.Width, a.Y+a.Height, a.X, a.Y, a.X+a.Width, a.Y)
		}
		objects = append(objects, fmt.Sprintf("<< /Type /Annot /Subtype /%s /Rect [%g %g %g %g] /Contents (%s)%s >>", kind, a.X, a.Y, a.X+a.Width, a.Y+a.Height, escape(a.Text), quad))
	}
	content := "BT /F1 12 Tf 20 100 Td (Bridge fixture) Tj ET\n"
	objects[3] = fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content)
	objects[2] = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [" + refs + "] >>"
	var out bytes.Buffer
	out.WriteString("%PDF-1.4\n")
	offsets := []int{0}
	for i, object := range objects {
		offsets = append(offsets, out.Len())
		fmt.Fprintf(&out, "%d 0 obj\n%s\nendobj\n", i+1, object)
	}
	start := out.Len()
	fmt.Fprintf(&out, "xref\n0 %d\n0000000000 65535 f \n", len(offsets))
	for _, offset := range offsets[1:] {
		fmt.Fprintf(&out, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&out, "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", len(offsets), start)
	return out.Bytes()
}

func (f *fakeApple) pdfCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) (bool, error) {
	f.mu.Lock()
	if frame.T == "Prepare" {
		if frame.Action == nil || frame.Action.Op != "pdf.annotate" {
			f.mu.Unlock()
			return false, nil
		}
		id := contracts.NewID("ticket")
		f.tickets[id] = ticket{CommandID: frame.CommandID, Action: *frame.Action, Hash: frame.RequestHash, Fence: fence, Expires: time.Now().Add(30 * time.Second)}
		f.mu.Unlock()
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: id, Fingerprint: frame.RequestHash, Route: "sim-pdf"})
		return true, err
	}
	t, ok := f.tickets[frame.Ticket]
	if frame.T != "Dispatch" || !ok || t.Action.Op != "pdf.annotate" {
		f.mu.Unlock()
		return false, nil
	}
	delete(f.tickets, frame.Ticket)
	if t.CommandID != frame.CommandID || t.Fence != fence || !time.Now().Before(t.Expires) {
		f.mu.Unlock()
		_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: "rejected_before_dispatch", Error: contracts.ErrExpired})
		return true, err
	}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, StartedAt: time.Now()}); err != nil {
		f.mu.Unlock()
		return true, err
	}
	raw, _ := json.Marshal(t.Action.Args)
	var args contracts.PDFAnnotateArgs
	json.Unmarshal(raw, &args)
	input, err := base64.StdEncoding.DecodeString(args.Data)
	if args.File != "" {
		file, openErr := os.Open(args.File)
		err = openErr
		if openErr == nil {
			info, statErr := file.Stat()
			if statErr != nil || !info.Mode().IsRegular() || info.Size() <= 0 || info.Size() > 8<<20 {
				err = contracts.ErrInvalid
			} else {
				input, err = io.ReadAll(io.LimitReader(file, (8<<20)+1))
			}
			file.Close()
		}
	}
	sum := sha256.Sum256(input)
	if err != nil || len(input) > 8<<20 || hex.EncodeToString(sum[:]) != "f71a9207e47bd6526a9a5a346541df4eb84de43649bcb11fce1ba7aa971b07cc" {
		f.mu.Unlock()
		_, err = wire.Send(contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "failed_no_effect"})
		return true, err
	}
	for _, a := range args.Annotations {
		if a.Page != 1 || a.X+a.Width > 600 || a.Y+a.Height > 800 {
			f.mu.Unlock()
			_, err = wire.Send(contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "failed_no_effect"})
			return true, err
		}
	}
	data := simulatedAnnotatedPDF(args.Annotations)
	sum = sha256.Sum256(data)
	digest := hex.EncodeToString(sum[:])
	f.blobs[digest] = data
	key := contracts.NewID("pdf")
	metadata, _ := json.Marshal(map[string]any{"blob": map[string]any{"digest": digest, "size": len(data), "state": "present"}, "annotation_count": len(args.Annotations)})
	observation := contracts.Observation{Kind: "upsert", Collection: "pdf.artifact", NativeKey: key, Data: metadata, ObservedAt: time.Now().UTC()}
	f.records[key] = observation
	err = f.changedLocked("pdf")
	revision := f.durable.Revision["pdf"]
	f.mu.Unlock()
	if err != nil {
		return true, err
	}
	if err = emit(contracts.Capture{Source: "pdf", Observations: []contracts.Observation{observation}, BlobHandles: map[string]string{digest: digest}}, revision); err != nil {
		return true, err
	}
	evidence, _ := json.Marshal(map[string]any{"native_key": key, "observed": true, "fingerprint": t.Hash, "capture_checkpoint": checkpoint("pdf"), "annotation_count": len(args.Annotations)})
	outcome := contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "locally_observed", Evidence: evidence}
	if err = f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, Outcome: &outcome}); err != nil {
		return true, err
	}
	_, err = wire.Send(outcome)
	return true, err
}

func init() { sourceCommands = append(sourceCommands, (*fakeApple).pdfCommand) }
