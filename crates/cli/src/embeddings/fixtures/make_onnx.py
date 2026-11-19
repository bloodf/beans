"""Writes the tiny ONNX fixtures the runtime-backed tests load (`tiny-text.onnx`,
`tiny-vision.onnx`), with the tensor contracts of a real image model's two towers:

- text: `input_ids`, `attention_mask` [batch, tokens] int64 -> `text_embeds` [batch, 4], a
  pooled mean of rows of a table over the fixture tokenizer's vocabulary (`tiny-tokenizer.json`);
- vision: `pixel_values` [batch, 3, 16, 16] float -> `image_embeds` [batch, 4], the mean of each
  channel through a 3x4 matrix.

They are built so the word `a` and a red picture point the same way, and `c` and a blue one:
a search test can tell real inference from luck. Run with `onnx` installed:

    python make_onnx.py
"""
import pathlib

import onnx
from onnx import TensorProto, helper

HERE = pathlib.Path(__file__).parent
# Rows by token id: [UNK], [PAD], a, b, c, d.
TABLE = [[0, 0, 0, 0], [0, 0, 0, 0], [1, 0, -1, 0], [0, 1, 0, 0], [-1, 0, 1, 0], [0, 0, 0, 1]]
# A red pixel normalizes to (1, -1, -1) with mean and std 0.5: through this, (1, 0, -1, 0).
CHANNELS = [[1, 0, 0, 0], [0, 0, 0, 0], [0, 0, 1, 0]]


def save(graph, name):
    model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 18)], producer_name="beans-fixtures")
    model.ir_version = 8
    onnx.checker.check_model(model)
    onnx.save(model, str(HERE / name))


def text():
    table = helper.make_tensor("table", TensorProto.FLOAT, [len(TABLE), 4], [v for row in TABLE for v in row])
    axes = helper.make_tensor("axes", TensorProto.INT64, [1], [1])
    nodes = [
        helper.make_node("Gather", ["table", "input_ids"], ["rows"]),
        # The mask is an input real exports take; multiplying by it keeps it in the graph.
        helper.make_node("Cast", ["attention_mask"], ["mask"], to=TensorProto.FLOAT),
        helper.make_node("Unsqueeze", ["mask", "last"], ["mask3"]),
        helper.make_node("Mul", ["rows", "mask3"], ["masked"]),
        helper.make_node("ReduceMean", ["masked", "axes"], ["text_embeds"], keepdims=0),
    ]
    last = helper.make_tensor("last", TensorProto.INT64, [1], [2])
    graph = helper.make_graph(
        nodes, "tiny-text",
        [helper.make_tensor_value_info("input_ids", TensorProto.INT64, ["batch", "tokens"]),
         helper.make_tensor_value_info("attention_mask", TensorProto.INT64, ["batch", "tokens"])],
        [helper.make_tensor_value_info("text_embeds", TensorProto.FLOAT, ["batch", 4])],
        [table, axes, last],
    )
    save(graph, "tiny-text.onnx")


def vision():
    weights = helper.make_tensor("weights", TensorProto.FLOAT, [3, 4], [v for row in CHANNELS for v in row])
    axes = helper.make_tensor("axes", TensorProto.INT64, [2], [2, 3])
    nodes = [
        helper.make_node("ReduceMean", ["pixel_values", "axes"], ["channels"], keepdims=0),
        helper.make_node("MatMul", ["channels", "weights"], ["image_embeds"]),
    ]
    graph = helper.make_graph(
        nodes, "tiny-vision",
        [helper.make_tensor_value_info("pixel_values", TensorProto.FLOAT, ["batch", 3, 16, 16])],
        [helper.make_tensor_value_info("image_embeds", TensorProto.FLOAT, ["batch", 4])],
        [weights, axes],
    )
    save(graph, "tiny-vision.onnx")


if __name__ == "__main__":
    text()
    vision()
