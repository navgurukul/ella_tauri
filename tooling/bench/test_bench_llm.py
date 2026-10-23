import json
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).parent))
import bench_llm


class StreamHandler(BaseHTTPRequestHandler):
    request_body = None
    status_code = 200

    def do_POST(self):
        length = int(self.headers["Content-Length"])
        StreamHandler.request_body = json.loads(self.rfile.read(length))
        if StreamHandler.status_code != 200:
            self.send_response(StreamHandler.status_code)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"error":"prompt is too long"}')
            return
        self.send_response(StreamHandler.status_code)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for event in (
            {"choices": [{"delta": {"content": "A kind "}}]},
            {"choices": [{"delta": {"content": "answer."}}]},
            {
                "choices": [],
                "usage": {"prompt_tokens": 125, "completion_tokens": 9},
                "timings": {
                    "prompt_n": 100,
                    "prompt_ms": 500,
                    "predicted_n": 9,
                    "predicted_ms": 900,
                },
            },
        ):
            self.wfile.write(f"data: {json.dumps(event)}\n\n".encode())
            self.wfile.flush()
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def log_message(self, _format, *_args):
        pass


class BenchLlmTests(unittest.TestCase):
    def setUp(self):
        StreamHandler.status_code = 200

    def test_dataset_is_fixed_and_hashed(self):
        dataset, digest = bench_llm.load_dataset(bench_llm.DEFAULT_DATASET)
        self.assertEqual(len(dataset["scenarios"][0]["turns"]), 6)
        self.assertEqual(len(digest), 64)

    def test_stream_request_matches_release_settings_and_collects_server_timings(self):
        server = HTTPServer(("127.0.0.1", 0), StreamHandler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)

        result = bench_llm.stream_turn(
            f"http://127.0.0.1:{server.server_port}/v1",
            [{"role": "system", "content": "system"}, {"role": "user", "content": "hello"}],
            slot=0,
            max_tokens=50,
            temperature=0.65,
            timeout=5,
        )

        self.assertEqual(
            {key: StreamHandler.request_body[key] for key in ("temperature", "max_tokens", "stream", "stream_options", "cache_prompt", "id_slot")},
            {
                "temperature": 0.65,
                "max_tokens": 50,
                "stream": True,
                "stream_options": {"include_usage": True},
                "cache_prompt": True,
                "id_slot": 0,
            },
        )
        self.assertEqual(result["response"], "A kind answer.")
        self.assertEqual(result["timings"]["predicted_n"], 9)
        self.assertEqual(result["usage"]["prompt_tokens"], 125)

    def test_compare_rejects_different_prompt_sets(self):
        reports = [
            {"dataset_sha256": "a", "settings": {}, "turns": []},
            {"dataset_sha256": "b", "settings": {}, "turns": []},
        ]
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / "first.json", Path(directory) / "second.json"]
            for path, report in zip(paths, reports):
                path.write_text(json.dumps(report), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "dataset differs"):
                bench_llm.compare([str(path) for path in paths])

    def test_compare_reports_missing_files_before_loading_reports(self):
        with tempfile.TemporaryDirectory() as directory:
            missing = str(Path(directory) / "not-created.json")
            with self.assertRaisesRegex(FileNotFoundError, "Run the benchmark successfully"):
                bench_llm.compare([missing, missing])

    def test_unreachable_server_has_actionable_error(self):
        with self.assertRaisesRegex(RuntimeError, "Start it first"):
            bench_llm.run(self.run_args("http://127.0.0.1:1/v1"))

    def test_unreachable_server_health_url_uses_base_url(self):
        with self.assertRaisesRegex(RuntimeError, "http://127.0.0.1:1/health"):
            bench_llm.run(self.run_args("http://127.0.0.1:1/v1"))

    def test_server_rejection_reports_http_status_and_body(self):
        StreamHandler.status_code = 400
        server = HTTPServer(("127.0.0.1", 0), StreamHandler)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        args = self.run_args(f"http://127.0.0.1:{server.server_port}/v1")
        args.timeout = 5

        with self.assertRaisesRegex(RuntimeError, r"HTTP 400.*prompt is too long"):
            bench_llm.run(args)

    @staticmethod
    def run_args(base_url):
        return SimpleNamespace(
            dataset=str(bench_llm.DEFAULT_DATASET),
            iterations=1,
            base_url=base_url,
            slot=0,
            max_tokens=50,
            temperature=0.65,
            timeout=0.1,
            candidate="test",
            model="test",
            runtime="test",
            threads=1,
            power_note=None,
            output=None,
        )


if __name__ == "__main__":
    unittest.main()
