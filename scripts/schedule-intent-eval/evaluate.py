"""Measures how well a local embedding model tells a request to repeat something ("run this daily",
"agende para rodar diariamente") from everything else, before Beans relies on one.

It trains a logistic-regression layer on the template messages in training.json and reports recall and false
positives on held_out.json, written separately by hand, splitting messages at punctuation so an ask in
the middle of a sentence counts. Thresholds chosen here are picked on the held-out set, so treat the
best row as optimistic.

Static models (a token embedding table, like WordLlama):
    python evaluate.py --tokenizer tokenizer.json --weights model.safetensors
ONNX sentence-embedding models (mean pooling over last_hidden_state, like multilingual-e5-small):
    python evaluate.py --tokenizer tokenizer.json --onnx model.onnx [--prefix "query: "]

Needs numpy, tokenizers, scikit-learn, and onnxruntime for --onnx. Nothing is downloaded.
"""
import argparse, json, re, struct
import numpy as np
from sklearn.linear_model import LogisticRegression
from tokenizers import Tokenizer


from training import generate, held_out

SPLIT = r"[,.;:!?，。！？、]"


def static_embedder(tokenizer, weights):
    raw = open(weights, "rb").read()
    size = struct.unpack("<Q", raw[:8])[0]
    header = json.loads(raw[8 : 8 + size])
    name = next(key for key in header if key != "__metadata__")
    meta = header[name]
    start, end = meta["data_offsets"]
    dtype = {"F16": np.float16, "F32": np.float32}[meta["dtype"]]
    table = np.frombuffer(raw[8 + size + start : 8 + size + end], dtype=dtype).reshape(meta["shape"]).astype(np.float32)

    def embed(texts):
        rows = []
        for text in texts:
            ids = tokenizer.encode(text, add_special_tokens=False).ids or [0]
            vector = table[np.clip(ids, 0, table.shape[0] - 1)].mean(0)
            rows.append(vector / np.linalg.norm(vector))
        return np.array(rows)

    return embed


def onnx_embedder(tokenizer, model, prefix):
    import onnxruntime

    session = onnxruntime.InferenceSession(model, providers=["CPUExecutionProvider"])
    inputs = {i.name for i in session.get_inputs()}

    def embed(texts):
        rows = []
        for text in texts:
            encoding = tokenizer.encode(prefix + text)
            feed = {"input_ids": np.array([encoding.ids], dtype=np.int64), "attention_mask": np.array([encoding.attention_mask], dtype=np.int64)}
            if "token_type_ids" in inputs:
                feed["token_type_ids"] = np.zeros_like(feed["input_ids"])
            hidden = session.run(None, feed)[0][0]
            mask = feed["attention_mask"][0][:, None]
            vector = (hidden * mask).sum(0) / mask.sum()
            rows.append(vector / np.linalg.norm(vector))
        return np.array(rows)

    return embed


def main():
    EVAL = held_out()
    parser = argparse.ArgumentParser()
    parser.add_argument("--tokenizer", required=True)
    parser.add_argument("--weights")
    parser.add_argument("--onnx")
    parser.add_argument("--prefix", default="")
    args = parser.parse_args()
    tokenizer = Tokenizer.from_file(args.tokenizer)
    embed = onnx_embedder(tokenizer, args.onnx, args.prefix) if args.onnx else static_embedder(tokenizer, args.weights)

    rows = generate()
    features = embed([message for _, _, message in rows])
    labels = np.array([label for _, label, _ in rows])
    positives = sum(label for _, label, _ in EVAL)
    for c in [1, 3, 10]:
        classifier = LogisticRegression(C=c, max_iter=5000, class_weight="balanced").fit(features, labels)

        def score(message):
            parts = [message] + [part.strip() for part in re.split(SPLIT, message) if part.strip()]
            return float(classifier.decision_function(embed(parts)).max())

        scored = [(language, label, score(message), message) for language, label, message in EVAL]
        for threshold in [-0.5, 0.0, 0.5]:
            found = sum(1 for _, label, s, _ in scored if label and s > threshold)
            wrong = sum(1 for _, label, s, _ in scored if not label and s > threshold)
            print(f"C={c:<3} threshold {threshold:+.1f}: recall {found}/{positives}, false positives {wrong}/{len(scored) - positives}")
    print("Errors at C=3, threshold 0:")
    classifier = LogisticRegression(C=3, max_iter=5000, class_weight="balanced").fit(features, labels)
    for language, label, message in EVAL:
        parts = [message] + [part.strip() for part in re.split(SPLIT, message) if part.strip()]
        s = float(classifier.decision_function(embed(parts)).max())
        if (s > 0) != bool(label):
            print(f"  {'missed' if label else 'false '} {language} {s:+.2f} {message}")


if __name__ == "__main__":
    main()
