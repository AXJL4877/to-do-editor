"""Small standard-library SDK for the todo-worker/1 line protocol.

The module deliberately has no third-party imports.  Extension authors only
register an operation and call ``Worker.serve()`` from their package entry
point; all protocol traffic remains valid even when an extension uses print().
"""
from __future__ import annotations

import json
import os
import sys
import threading
import traceback
from concurrent.futures import Future
from pathlib import Path
from typing import Any, Callable, Dict, Optional


class ServiceError(Exception):
    def __init__(self, cls: str, message: str):
        super().__init__(message)
        self.cls = cls
        self.message = message


class RunFailed(Exception):
    def __init__(self, message: str, cls: str = "internal"):
        super().__init__(message)
        self.cls = cls
        self.message = message


class _Reply:
    def __init__(self) -> None:
        self.event = threading.Event()
        self.value: Any = None
        self.error: Optional[dict] = None


class Context:
    def __init__(self, worker: "Worker", message: dict, cancelled: threading.Event):
        self._worker = worker
        self._message = message
        self._cancel_event = cancelled
        self.input = message.get("input")
        self.params = message.get("params")
        self.canvas = message.get("canvas")
        self.workdir = Path(message["workdir"])
        self.seed = message.get("seed", 0)

    def cancelled(self) -> bool:
        return self._cancel_event.is_set()

    def check_cancelled(self) -> None:
        """Stop a long loop promptly after the host cancelled the run."""
        if self.cancelled():
            raise RunFailed("cancelled", "cancelled")

    def progress(self, phase: str, fraction: float) -> None:
        self._worker._send({"type": "progress", "run": self._message["id"],
                            "phase": str(phase), "fraction": float(fraction)})

    def emit(self, items: list) -> None:
        if not isinstance(items, list):
            raise RunFailed("emit expects a list", "invalid_input")
        self._worker._send({"type": "emit", "run": self._message["id"], "items": items})

    def call(self, service: str, request: dict) -> Any:
        if self.cancelled():
            raise RunFailed("cancelled", "cancelled")
        reply = self._worker._call(self._message["id"], service, request)
        reply.event.wait()
        if self.cancelled():
            raise RunFailed("cancelled", "cancelled")
        if reply.error is not None:
            raise ServiceError(reply.error.get("class", "unavailable"),
                               reply.error.get("message", "service failed"))
        return reply.value

    def call_async(self, service: str, request: dict) -> Future:
        future: Future = Future()

        def run() -> None:
            try:
                future.set_result(self.call(service, request))
            except BaseException as exc:  # Future preserves the worker error.
                future.set_exception(exc)

        threading.Thread(target=run, name="todo-worker-call", daemon=True).start()
        return future

    def path(self, relative: str) -> Path:
        if not isinstance(relative, str) or not relative or "\x00" in relative:
            raise RunFailed("path must be a non-empty relative string", "invalid_input")
        candidate = (self.workdir / relative).resolve()
        root = self.workdir.resolve()
        try:
            candidate.relative_to(root)
        except ValueError as exc:
            raise RunFailed("path escapes workdir", "invalid_input") from exc
        candidate.parent.mkdir(parents=True, exist_ok=True)
        return candidate


class Worker:
    def __init__(self, version: str = "1.0.0"):
        self.version = version
        self._ops: Dict[str, Callable[[Context], Any]] = {}
        self._write_lock = threading.Lock()
        self._replies: Dict[str, _Reply] = {}
        self._reply_lock = threading.Lock()
        self._cancel: Dict[str, threading.Event] = {}
        self._threads: Dict[str, threading.Thread] = {}
        # Keep a binary fd for protocol output.  Author output goes to stderr.
        self._protocol = os.fdopen(os.dup(1), "w", encoding="utf-8", buffering=1)
        self._author_stdout = sys.stdout
        sys.stdout = sys.stderr

    def op(self, name: str) -> Callable[[Callable[[Context], Any]], Callable[[Context], Any]]:
        def register(fn: Callable[[Context], Any]) -> Callable[[Context], Any]:
            if not name or name in self._ops:
                raise ValueError("operation name must be unique and non-empty")
            self._ops[name] = fn
            return fn
        return register

    def _send(self, value: dict) -> None:
        with self._write_lock:
            self._protocol.write(json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n")
            self._protocol.flush()

    def _call(self, run: str, service: str, request: dict) -> _Reply:
        call_id = f"{run}:c{self._next_call()}"
        reply = _Reply()
        with self._reply_lock:
            self._replies[call_id] = reply
        self._send({"type": "call", "id": call_id, "run": run,
                    "service": service, "request": request})
        return reply

    _call_counter = 0
    _counter_lock = threading.Lock()

    @classmethod
    def _next_call(cls) -> int:
        with cls._counter_lock:
            cls._call_counter += 1
            return cls._call_counter

    def _run(self, message: dict) -> None:
        run_id = message["id"]
        cancel = threading.Event()
        self._cancel[run_id] = cancel
        try:
            fn = self._ops.get(message.get("op"))
            if fn is None:
                raise RunFailed("unknown operation", "invalid_input")
            result = fn(Context(self, message, cancel))
            if cancel.is_set():
                raise RunFailed("cancelled", "cancelled")
            self._send({"type": "done", "run": run_id, "result": result})
        except RunFailed as exc:
            self._send({"type": "failed", "run": run_id,
                        "error": {"class": exc.cls, "message": exc.message}})
        except ServiceError as exc:
            self._send({"type": "failed", "run": run_id,
                        "error": {"class": "internal", "message": str(exc)}})
        except MemoryError:
            self._send({"type": "failed", "run": run_id,
                        "error": {"class": "resource", "message": "out of memory"}})
        except OSError as exc:
            cls = "resource" if exc.errno == 28 else "internal"
            self._send({"type": "failed", "run": run_id,
                        "error": {"class": cls, "message": str(exc)}})
        except BaseException:
            lines = traceback.format_exc().strip().splitlines()
            message_text = lines[-1] if lines else "worker failed"
            self._send({"type": "failed", "run": run_id,
                        "error": {"class": "internal", "message": message_text}})
        finally:
            self._cancel.pop(run_id, None)

    def serve(self) -> None:
        first = self._read()
        if first.get("type") != "hello":
            raise RuntimeError("expected hello")
        self._send({"type": "ready", "spec": "todo-worker/1",
                    "ops": sorted(self._ops), "version": self.version})
        for message in sys.stdin:
            if not message.strip():
                continue
            value = json.loads(message)
            kind = value.get("type")
            if kind == "run":
                thread = threading.Thread(target=self._run, args=(value,), daemon=True)
                self._threads[value["id"]] = thread
                thread.start()
            elif kind == "reply":
                with self._reply_lock:
                    reply = self._replies.pop(value.get("to", ""), None)
                if reply is not None:
                    reply.value = value.get("result")
                    reply.error = value.get("error") if not value.get("ok", False) else None
                    reply.event.set()
            elif kind == "cancel":
                event = self._cancel.get(value.get("id", ""))
                if event is not None:
                    event.set()
            elif kind == "shutdown":
                for event in self._cancel.values():
                    event.set()
                break

    @staticmethod
    def _read() -> dict:
        line = sys.stdin.readline()
        if not line:
            raise RuntimeError("host closed the protocol")
        return json.loads(line)

