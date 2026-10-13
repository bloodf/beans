package main

import (
	"encoding/json"
	"errors"
	"github.com/egoist/mygo/ui"
	"net/url"
	"strconv"
	"strings"
)

type memoryEmbeddingRow struct {
	ID         string `json:"id"`
	Model      string `json:"model"`
	Revision   string `json:"model_revision"`
	Dimensions int    `json:"dimensions"`
	HasSecret  bool   `json:"has_secret"`
}
type memoryEmbeddingForm struct {
	Existing                     bool
	ID, Mode, SecretMode, Secret string
	Values                       map[string]string
	Remove                       bool
}

func (n *nativeDesktop) editMemoryEmbedding(row *memoryEmbeddingRow) {
	f := &n.memoryPanel.Forms
	f.serial++
	f.Error = ""
	d := &memoryEmbeddingForm{Mode: "api", SecretMode: "keep", Values: map[string]string{"normalization": "l2", "distance": "cosine", "pooling": "mean", "max_tokens": "512", "pad_id": "0", "pad_type_id": "0", "pad_token": "[PAD]", "add_special_tokens": "true"}}
	if row != nil {
		d.Existing = true
		d.ID = row.ID
		d.Values["model"] = row.Model
		d.Values["revision"] = row.Revision
		d.Values["dimensions"] = strconv.Itoa(row.Dimensions)
	}
	f.Embedding = d
}
func (n *nativeDesktop) embeddingParams(d *memoryEmbeddingForm) (json.RawMessage, error) {
	v := d.Values
	if d.ID == "" {
		return nil, errors.New("Profile ID is required")
	}
	if len(v["model"]) < 1 || len(v["model"]) > 1024 || len(v["revision"]) < 1 || len(v["revision"]) > 1024 || len(v["document_prefix"]) > 4096 || len(v["query_prefix"]) > 4096 {
		return nil, errors.New("Invalid vector space text")
	}
	if v["normalization"] != "none" && v["normalization"] != "l2" {
		return nil, errors.New("Invalid normalization")
	}
	if v["distance"] != "cosine" && v["distance"] != "dot" && v["distance"] != "euclidean" {
		return nil, errors.New("Invalid distance")
	}
	profile := map[string]any{"model": v["model"], "revision": v["revision"], "normalization": v["normalization"], "distance": v["distance"], "document_prefix": v["document_prefix"], "query_prefix": v["query_prefix"], "mode": d.Mode, "endpoint": nil, "local": nil}
	number := func(key string) (int, error) {
		n, e := strconv.Atoi(v[key])
		if e != nil {
			return 0, errors.New("Invalid " + key)
		}
		return n, nil
	}
	dimensions, err := number("dimensions")
	if err != nil {
		return nil, err
	}
	profile["dimensions"] = dimensions
	if dimensions < 1 || dimensions > 65536 {
		return nil, errors.New("Invalid dimensions")
	}
	if d.Mode == "api" {
		u, err := url.Parse(v["endpoint"])
		if err != nil || (u.Scheme != "https" && u.Scheme != "http") || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || !strings.HasSuffix(u.Path, "/embeddings") {
			return nil, errors.New("Invalid embeddings endpoint")
		}
		profile["endpoint"] = v["endpoint"]
	} else if d.Mode == "local_cpu" {
		for _, key := range []string{"model_sha256", "tokenizer_sha256"} {
			hash := v[key]
			if len(hash) != 64 || strings.IndexFunc(hash, func(r rune) bool { return !strings.ContainsRune("0123456789abcdef", r) }) >= 0 {
				return nil, errors.New("Invalid artifact hash")
			}
		}
		local := map[string]any{"model_sha256": v["model_sha256"], "tokenizer_sha256": v["tokenizer_sha256"], "pooling": v["pooling"], "pad_token": v["pad_token"], "add_special_tokens": v["add_special_tokens"] != "false"}
		for _, key := range []string{"max_tokens", "pad_id", "pad_type_id"} {
			value, e := number(key)
			if e != nil {
				return nil, e
			}
			local[key] = value
		}
		if local["max_tokens"].(int) < 1 || local["max_tokens"].(int) > 8192 || local["pad_id"].(int) < 0 || local["pad_type_id"].(int) < 0 || len(v["pad_token"]) > 256 {
			return nil, errors.New("Invalid local token settings")
		}
		if v["pooling"] != "mean" && v["pooling"] != "cls" && v["pooling"] != "pooled" {
			return nil, errors.New("Invalid pooling")
		}
		for _, key := range []string{"input_ids", "attention_mask", "output"} {
			if v[key] == "" {
				return nil, errors.New("Missing tensor name")
			}
		}
		var tokenType any
		if v["token_type_ids"] != "" {
			tokenType = v["token_type_ids"]
		}
		local["tensors"] = map[string]any{"input_ids": v["input_ids"], "attention_mask": v["attention_mask"], "token_type_ids": tokenType, "output": v["output"]}
		profile["local"] = local
	} else {
		return nil, errors.New("Invalid execution mode")
	}
	secretMode := d.SecretMode
	if d.Mode == "local_cpu" {
		secretMode = "clear"
	}
	var secret json.RawMessage
	err = n.memory.runtime.eval(`if(!["keep","replace","clear"].includes(input.action))throw new Error("invalid_secret");return BeansMemory.secretPatch(input.action,input.value);`, map[string]string{"action": secretMode, "value": d.Secret}, &secret)
	if err != nil {
		return nil, errors.New("Invalid secret patch")
	}
	return jsonBytes(map[string]any{"id": d.ID, "profile": profile, "secret": secret}), nil
}
func (n *nativeDesktop) saveMemoryEmbedding() {
	f := &n.memoryPanel.Forms
	d := f.Embedding
	if d == nil {
		return
	}
	params, err := n.embeddingParams(d)
	if err != nil {
		f.Error = err.Error()
		return
	}
	n.memoryCall("memory.embeddings.set", params, func(_ json.RawMessage, err error) {
		if err != nil {
			f.Error = "Embedding profile could not be saved. Review the complete vector space and retry."
			return
		}
		d.Secret = ""
		f.Embedding = nil
		f.serial++
		n.loadMemory()
	})
}
func (n *nativeDesktop) embeddingFormView(c *ui.Context) {
	f := &n.memoryPanel.Forms
	d := f.Embedding
	ui.Text(c, "Embedding profile").Bold()
	ui.Text(c, "Pin the complete vector space. Changing it requires a validated replacement index. Masked reads do not return endpoint or local settings: supply the complete replacement profile before saving.")
	if f.Error != "" {
		ui.Text(c, f.Error)
	}
	ui.Scroll(c).Grow(1).Children(func() {
		ui.Column(c).Gap(8).Disabled(f.Pending).Children(func() {
			ui.TextInput(c, &d.ID).Label("Profile ID").Disabled(d.Existing)
			for _, field := range [][2]string{{"model", "Exact model"}, {"revision", "Immutable model revision"}, {"dimensions", "Dimensions"}} {
				value := d.Values[field[0]]
				if ui.TextInput(c, &value).Label(field[1]).Changed() {
					d.Values[field[0]] = value
				}
			}
			if ui.Select(c, &d.Mode, []string{"api", "local_cpu"}).Label("Execution").Changed() {
				d.Secret = ""
			}
			for _, field := range [][2]string{{"normalization", "Normalization"}, {"distance", "Distance"}, {"document_prefix", "Exact document prefix"}, {"query_prefix", "Exact query prefix"}} {
				value := d.Values[field[0]]
				if field[0] == "normalization" {
					if ui.Select(c, &value, []string{"none", "l2"}).Label(field[1]).Changed() {
						d.Values[field[0]] = value
					}
				} else if field[0] == "distance" {
					if ui.Select(c, &value, []string{"cosine", "dot", "euclidean"}).Label(field[1]).Changed() {
						d.Values[field[0]] = value
					}
				} else if ui.TextInput(c, &value).Label(field[1]).Changed() {
					d.Values[field[0]] = value
				}
			}
			fields := [][2]string{{"endpoint", "Exact /embeddings endpoint"}}
			if d.Mode == "local_cpu" {
				fields = [][2]string{{"model_sha256", "Model SHA-256"}, {"tokenizer_sha256", "Tokenizer SHA-256"}, {"max_tokens", "Maximum tokens"}, {"input_ids", "Input IDs tensor"}, {"attention_mask", "Attention mask tensor"}, {"token_type_ids", "Token type tensor (optional)"}, {"output", "Output tensor"}, {"pooling", "Pooling"}, {"pad_id", "Padding ID"}, {"pad_type_id", "Padding type ID"}, {"pad_token", "Padding token"}, {"add_special_tokens", "Add special tokens (true/false)"}}
			}
			for _, field := range fields {
				value := d.Values[field[0]]
				if field[0] == "pooling" {
					if ui.Select(c, &value, []string{"mean", "cls", "pooled"}).Label(field[1]).Changed() {
						d.Values[field[0]] = value
					}
				} else if field[0] == "add_special_tokens" {
					enabled := value != "false"
					if ui.Checkbox(c, &enabled, "Add special tokens").Changed() {
						d.Values[field[0]] = strconv.FormatBool(enabled)
					}
				} else if ui.TextInput(c, &value).Label(field[1]).Changed() {
					d.Values[field[0]] = value
				}
			}
			if ui.Select(c, &d.SecretMode, []string{"keep", "replace", "clear"}).Label("Embedding secret change").Changed() {
				d.Secret = ""
			}
			if d.SecretMode == "replace" {
				ui.TextInput(c, &d.Secret).Password().Label("Replacement embedding secret")
			}
		})
	})
	ui.Row(c).Gap(8).Children(func() {
		if ui.Button(c, "Save profile").Disabled(f.Pending).Clicked() {
			n.saveMemoryEmbedding()
		}
		if ui.Button(c, "Cancel").Disabled(f.Pending).Clicked() {
			d.Secret = ""
			f.Embedding = nil
			f.serial++
		}
		if d.Existing && ui.Button(c, "Remove profile…").Disabled(f.Pending).Clicked() {
			d.Remove = true
		}
		if d.Remove {
			ui.Text(c, "Remove this embedding profile?")
			if ui.Button(c, "Confirm remove profile").Disabled(f.Pending).Clicked() {
				n.memoryCall("memory.embeddings.remove", jsonBytes(map[string]string{"id": d.ID}), func(_ json.RawMessage, err error) {
					if err != nil {
						f.Error = "Embedding profile could not be removed."
						return
					}
					d.Secret = ""
					f.Embedding = nil
					f.serial++
					n.loadMemory()
				})
			}
		}
	})
}
