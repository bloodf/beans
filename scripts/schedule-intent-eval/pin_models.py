"""Pins the schedule-model catalog (crates/cli/src/schedule_models/catalog.json) so Runners can
download its models: each file's exact size and SHA-256, and the revision as the commit it was
read at, so the download URL never changes under the pin.

    python pin_models.py                      # every model not pinned yet, from Hugging Face
    python pin_models.py --model potion-base-8m --model all-minilm-l6-v2-int8
    python pin_models.py --all                # re-read every model, pinned ones included
    python pin_models.py --offline DIR        # from files already downloaded into DIR/<repo>/<path>

It downloads each file (set HF_TOKEN for a gated repository), hashes it, and checks it against the
SHA-256 Hugging Face reports for large files. For an ONNX model with onnxruntime installed, it also
checks the model's tensors against the catalog entry (both towers of an image model). It refuses a
model over 1 GB, and changes nothing unless every file of that model reads cleanly. Review the diff
before committing it.
"""
import argparse, hashlib, json, os, pathlib, sys, tempfile, urllib.parse, urllib.request

CATALOG = pathlib.Path(__file__).resolve().parents[2] / "crates" / "cli" / "src" / "schedule_models" / "catalog.json"
MAX_MODEL_BYTES = 1_000_000_000


def request(url):
    headers = {"User-Agent": "beans-pin-models"}
    if os.environ.get("HF_TOKEN"):
        headers["Authorization"] = f"Bearer {os.environ['HF_TOKEN']}"
    return urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=60)


def files_of(model):
    """The model's files by role, in the catalog's order (`CatalogModel::files`)."""
    graphs = {"static": ["weights"], "onnx": ["model"], "clip": ["model", "image_model"]}[model["engine"]]
    return [(role, model["files"][role]) for role in graphs] + [("tokenizer", model["files"]["tokenizer"])]


def commit_and_lfs(endpoint, model):
    """The commit `revision` names now, and the SHA-256 Hugging Face lists for its LFS files."""
    url = f"{endpoint}/api/models/{model['repo']}/revision/{urllib.parse.quote(model['revision'], safe='')}?blobs=true"
    with request(url) as response:
        info = json.load(response)
    lfs = {s["rfilename"]: s["lfs"]["sha256"] for s in info.get("siblings", []) if s.get("lfs", {}).get("sha256")}
    return info["sha"], lfs


def read(source, path_out):
    """Copies a URL or a local file to `path_out`, returning its size and SHA-256."""
    digest, size = hashlib.sha256(), 0
    opened = request(source) if source.startswith(("http://", "https://")) else open(source, "rb")
    with opened as stream, open(path_out, "wb") as out:
        while chunk := stream.read(1 << 20):
            size += len(chunk)
            if size > MAX_MODEL_BYTES:
                raise ValueError("more than 1 GB")
            digest.update(chunk)
            out.write(chunk)
    if size == 0:
        raise ValueError("empty file")  # the Runner's catalog check refuses a 0-byte pin
    return size, digest.hexdigest()


def check_onnx(model, path):
    """The graph's tensors against the catalog's contract, when onnxruntime is there to read it."""
    try:
        import onnxruntime
    except ImportError:
        print("  (onnxruntime not installed: tensor contract not checked)")
        return
    spec = model["onnx"]
    session = onnxruntime.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    wanted = {spec["tensors"]["input_ids"], spec["tensors"]["attention_mask"]} | ({spec["tensors"]["token_type_ids"]} if spec["tensors"]["token_type_ids"] else set())
    inputs = {i.name: i for i in session.get_inputs()}
    if set(inputs) != wanted or any(i.type != "tensor(int64)" for i in inputs.values()):
        raise ValueError(f"inputs {sorted((n, i.type) for n, i in inputs.items())} are not {sorted(wanted)} as int64")
    outputs = {o.name: o for o in session.get_outputs()}
    output = outputs.get(spec["tensors"]["output"])
    if output is None or output.type != "tensor(float)" or len(output.shape) != (2 if spec["pooling"] == "pooled" else 3):
        raise ValueError(f"output {spec['tensors']['output']} missing or not float of the pooling's rank: {[(o.name, o.type, o.shape) for o in outputs.values()]}")
    if isinstance(output.shape[-1], int) and output.shape[-1] != spec["dimensions"]:
        raise ValueError(f"output width {output.shape[-1]} is not {spec['dimensions']}")


def check_image_onnx(model, path):
    """An image model's vision tower: one float [batch, 3, size, size] input, a float
    [batch, dimensions] output (`embeddings::image::validate_session`)."""
    try:
        import onnxruntime
    except ImportError:
        print("  (onnxruntime not installed: image tensor contract not checked)")
        return
    spec, width = model["image"], model["onnx"]["dimensions"]
    session = onnxruntime.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    inputs = session.get_inputs()
    fits = lambda shape, want: len(shape) == len(want) and all(not isinstance(s, int) or s in (-1, w) for s, w in zip(shape, want))
    if len(inputs) != 1 or inputs[0].name != spec["input"] or inputs[0].type != "tensor(float)" or not fits(inputs[0].shape, [-1, 3, spec["size"], spec["size"]]):
        raise ValueError(f"inputs {[(i.name, i.type, i.shape) for i in inputs]} are not one float {spec['input']} [batch, 3, {spec['size']}, {spec['size']}]")
    outputs = {o.name: o for o in session.get_outputs()}
    output = outputs.get(spec["output"])
    if output is None or output.type != "tensor(float)" or not fits(output.shape, [-1, width]):
        raise ValueError(f"output {spec['output']} missing or not float [batch, {width}]: {[(o.name, o.type, o.shape) for o in outputs.values()]}")


def pin(model, endpoint, offline):
    pins, revision, lfs = {}, model["revision"], {}
    if not offline:
        revision, lfs = commit_and_lfs(endpoint, model)
    with tempfile.TemporaryDirectory() as scratch:
        for role, file in files_of(model):
            source = str(pathlib.Path(offline) / model["repo"] / file["path"]) if offline else f"{endpoint}/{model['repo']}/resolve/{revision}/{file['path']}"
            local = pathlib.Path(scratch) / role
            size, sha256 = read(source, local)
            if file["path"] in lfs and lfs[file["path"]] != sha256:
                raise ValueError(f"{file['path']}: Hugging Face lists {lfs[file['path']]}, the download hashes to {sha256}")
            if role == "model":
                check_onnx(model, local)
            if role == "image_model":
                check_image_onnx(model, local)
            pins[role] = (size, sha256)
    total = sum(size for size, _ in pins.values())
    if total > MAX_MODEL_BYTES:
        raise ValueError(f"{total} bytes, more than 1 GB")
    return revision, pins


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--model", action="append", help="a catalog id; repeat for several")
    parser.add_argument("--all", action="store_true", help="pinned models too")
    parser.add_argument("--offline", help="read files from DIR/<repo>/<path> instead of downloading")
    parser.add_argument("--endpoint", default="https://huggingface.co")
    parser.add_argument("--catalog", default=str(CATALOG))
    args = parser.parse_args()
    catalog = json.loads(pathlib.Path(args.catalog).read_text())
    pinned = lambda m: all(f["bytes"] is not None and f["sha256"] is not None for _, f in files_of(m))
    chosen = [m for m in catalog["models"] if (m["id"] in args.model if args.model else args.all or not pinned(m))]
    if args.model and len(chosen) != len(set(args.model)):
        sys.exit(f"unknown model in {args.model}")
    failed = any_changed = False
    for model in chosen:
        print(f"{model['id']} ({model['repo']} @ {model['revision']})")
        try:
            revision, pins = pin(model, args.endpoint.rstrip("/"), args.offline)
        except Exception as error:  # one model's trouble leaves the others to pin
            print(f"  not pinned: {error}")
            failed = True
            continue
        changed = False
        for role, file in files_of(model):
            size, sha256 = pins[role]
            if (file["bytes"], file["sha256"]) != (size, sha256):
                if file["sha256"] is not None:
                    print(f"  {role} changed from {file['bytes']} bytes {file['sha256']}")
                file["bytes"], file["sha256"], changed = size, sha256, True
            print(f"  {role}: {file['path']} {size} bytes sha256 {sha256}")
        if revision != model["revision"]:
            model["revision"], changed = revision, True
            print(f"  revision: {revision}")
        if changed:
            model.pop("approx_bytes", None)
            model["pinned_from"] = "local files" if args.offline else f"{args.endpoint} at commit {revision}"
            any_changed = True
    if any_changed:
        pathlib.Path(args.catalog).write_text(json.dumps(catalog, indent=2, ensure_ascii=False) + "\n")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
