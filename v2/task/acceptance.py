#!/usr/bin/env python3
"""Black-box acceptance tests for miniq (SPEC.md, and SPEC-feature.md with --feature).

Usage: python3 -I acceptance.py <path-to-miniq-binary> [--feature] [--json] [-k SUBSTRING]

Every scenario starts a fresh process, feeds it a script of commands and compares the responses
with the expected ones member by member. `error.message` is never compared and extra members are
accepted inside `error` objects only.
"""

import json
import queue
import subprocess
import sys
import threading
import time

TIMEOUT = 10.0
FEATURE = False
SCENARIOS = []

MAX_QUEUES = 1000
MAX_MESSAGES = 100000


class ErrorObject:
    def __init__(self, code):
        self.code = code

    def __repr__(self):
        return "error(%s)" % self.code


def err(code):
    return {"ok": False, "error": ErrorObject(code)}


def ok(**members):
    response = {"ok": True}
    response.update(members)
    return response


OK = ok()
INVALID_REQUEST = err("InvalidRequest")
MISSING = err("MissingParameter")
INVALID_VALUE = err("InvalidParameterValue")
NAME_INVALID = err("QueueNameInvalid")
ALREADY_EXISTS = err("QueueAlreadyExists")
NO_QUEUE = err("QueueDoesNotExist")
HANDLE_INVALID = err("ReceiptHandleInvalid")
HANDLE_EXPIRED = err("ReceiptHandleExpired")
UNSUPPORTED = err("UnsupportedOperation")
LIMIT = err("LimitExceeded")
EMPTY_BATCH = err("EmptyBatchRequest")
TOO_MANY_ENTRIES = err("TooManyEntriesInBatchRequest")
INVALID_ENTRY_ID = err("InvalidBatchEntryId")
IDS_NOT_DISTINCT = err("BatchEntryIdsNotDistinct")


def attrs(**overrides):
    attributes = {
        "VisibilityTimeout": 30,
        "DelaySeconds": 0,
        "RetentionSeconds": 345600,
        "Fifo": False,
        "ContentBasedDeduplication": False,
    }
    if FEATURE:
        attributes["MaxMessageBytes"] = 1024
    attributes.update(overrides)
    return attributes


def counts(visible=0, in_flight=0, delayed=0):
    return {"Visible": visible, "InFlight": in_flight, "Delayed": delayed}


def handle_text(handle):
    return "r-%d" % handle if isinstance(handle, int) else handle


def msg(message, handle, body, count=1, sent=0, first=0, group=None, seq=None):
    delivered = {
        "MessageId": "m-%d" % message,
        "ReceiptHandle": "r-%d" % handle,
        "Body": body,
        "ReceiveCount": count,
        "SentAt": sent,
        "FirstReceivedAt": first,
    }
    if group is not None:
        delivered["GroupId"] = group
        delivered["SequenceNumber"] = seq
    return delivered


def failed(entry_id, response):
    return {"Id": entry_id, "error": response["error"]}


def mismatch(expected, actual, path="$"):
    if isinstance(expected, ErrorObject):
        if not isinstance(actual, dict):
            return "%s: expected an error object, got %r" % (path, actual)
        if actual.get("code") != expected.code:
            return "%s.code: expected %r, got %r" % (path, expected.code, actual.get("code"))
        if not isinstance(actual.get("message"), str):
            return "%s.message: expected a string, got %r" % (path, actual.get("message"))
        return None
    if isinstance(expected, dict):
        if not isinstance(actual, dict):
            return "%s: expected an object, got %r" % (path, actual)
        for key in expected:
            if key not in actual:
                return "%s: member %r is missing" % (path, key)
        for key in actual:
            if key not in expected:
                return "%s: unexpected member %r" % (path, key)
        for key, value in expected.items():
            found = mismatch(value, actual[key], "%s.%s" % (path, key))
            if found:
                return found
        return None
    if isinstance(expected, list):
        if not isinstance(actual, list):
            return "%s: expected an array, got %r" % (path, actual)
        if len(expected) != len(actual):
            return "%s: expected %d elements, got %d" % (path, len(expected), len(actual))
        for index, value in enumerate(expected):
            found = mismatch(value, actual[index], "%s[%d]" % (path, index))
            if found:
                return found
        return None
    if type(expected) is not type(actual) or expected != actual:
        return "%s: expected %s, got %s" % (path, short(expected), short(actual))
    return None


def short(value):
    text = json.dumps(value) if not isinstance(value, ErrorObject) else repr(value)
    return text if len(text) <= 80 else text[:77] + "..."


class Script:
    def __init__(self):
        self.steps = []
        self.final_newline = True
        self.known = {}

    def raw(self, line, expect, label="raw line"):
        if isinstance(line, str):
            line = line.encode("utf-8")
        self.steps.append((label, line, expect))

    def cmd(self, command, expect=None, **params):
        request = {"Command": command}
        request.update(params)
        self.raw(json.dumps(request), OK if expect is None else expect, command)

    def create(self, name, expect=None, **attributes):
        params = {"QueueName": name}
        if attributes:
            params["Attributes"] = attributes
        self.cmd("CreateQueue", expect, **params)
        if expect is None and name not in self.known:
            self.known[name] = attrs(**attributes)

    def set(self, name, expect=None, **attributes):
        self.cmd("SetQueueAttributes", expect, QueueName=name, Attributes=attributes)
        if expect is None:
            self.known[name].update(attributes)

    def drop(self, name, expect=None):
        self.cmd("DeleteQueue", expect, QueueName=name)
        if expect is None:
            del self.known[name]
            for attributes in self.known.values():
                if attributes.get("DeadLetterQueue") == name:
                    del attributes["DeadLetterQueue"]
                    del attributes["MaxReceiveCount"]

    def counts(self, name, visible=0, in_flight=0, delayed=0):
        expect = ok(Attributes=dict(self.known[name]), Counts=counts(visible, in_flight, delayed))
        self.cmd("GetQueueAttributes", expect, QueueName=name)

    def send(self, name, body, message=None, seq=None, expect=None, **params):
        if expect is None:
            expect = ok(MessageId="m-%d" % message)
            if seq is not None:
                expect["SequenceNumber"] = seq
        self.cmd("SendMessage", expect, QueueName=name, Body=body, **params)

    def fsend(self, name, body, group, dedup, message=None, seq=None, expect=None):
        self.send(name, body, message, seq, expect, GroupId=group, DeduplicationId=dedup)

    def receive(self, name, messages=None, expect=None, **params):
        if expect is None:
            expect = ok(Messages=messages)
        self.cmd("ReceiveMessage", expect, QueueName=name, **params)

    def delete(self, name, handle, expect=None):
        self.cmd("DeleteMessage", expect, QueueName=name, ReceiptHandle=handle_text(handle))

    def change(self, name, handle, timeout, expect=None):
        self.cmd(
            "ChangeMessageVisibility",
            expect,
            QueueName=name,
            ReceiptHandle=handle_text(handle),
            VisibilityTimeout=timeout,
        )

    def purge(self, name, expect=None):
        self.cmd("PurgeQueue", expect, QueueName=name)

    def tick(self, seconds, now):
        self.cmd("Tick", ok(Now=now), Seconds=seconds)

    def list(self, names, **params):
        self.cmd("ListQueues", ok(QueueNames=names), **params)

    def send_batch(self, name, entries, expect):
        self.cmd("SendMessageBatch", expect, QueueName=name, Entries=entries)

    def delete_batch(self, name, entries, expect):
        self.cmd("DeleteMessageBatch", expect, QueueName=name, Entries=entries)

    def tag(self, name, tags, expect=None):
        self.cmd("TagQueue", expect, QueueName=name, Tags=tags)

    def untag(self, name, keys, expect=None):
        self.cmd("UntagQueue", expect, QueueName=name, TagKeys=keys)

    def tags(self, name, tags):
        self.cmd("ListQueueTags", ok(Tags=dict(tags)), QueueName=name)


def scenario(function=None, feature=False, interactive=False):
    def register(function):
        SCENARIOS.append((function.__name__, function, feature, interactive))
        return function

    return register(function) if function else register


# Protocol


@scenario
def protocol_malformed_lines(s):
    for line in [
        "this is not json",
        "",
        "   ",
        "{",
        '{"Command": "ListQueues"',
        "[]",
        '["ListQueues"]',
        '"ListQueues"',
        "42",
        "null",
        "true",
        "{}",
        '{"Command": 7}',
        '{"Command": null}',
        '{"Command": ["ListQueues"]}',
        '{"Command": {"Name": "ListQueues"}}',
        '{"Command": ""}',
        '{"Command": "Explode"}',
        '{"Command": "listqueues"}',
        '{"Command": "LISTQUEUES"}',
        '{"command": "ListQueues"}',
        '{"Command": "ListQueues"} trailing',
        '{"Command": "ListQueues"}{"Command": "ListQueues"}',
        "{'Command': 'ListQueues'}",
        '{"Command": "ListQueues",}',
    ]:
        s.raw(line, INVALID_REQUEST)
    s.list([])
    s.raw('  {"Command" : "ListQueues" }  ', ok(QueueNames=[]))
    s.raw('{"QueueName": "q", "Command": "CreateQueue"}', OK)
    s.raw("nonsense again", INVALID_REQUEST)
    s.list(["q"])


@scenario
def protocol_invalid_utf8_line(s):
    s.create("q")
    s.raw(b"\xff\xfe\xfd", INVALID_REQUEST)
    s.raw(b'{"Command": "SendMessage", "QueueName": "q", "Body": "\xc3\x28"}', INVALID_REQUEST)
    s.raw(b'{"Command": "ListQueues", "Prefix": "\x80"}', INVALID_REQUEST)
    s.send("q", "still alive", 1)
    s.counts("q", 1, 0, 0)


@scenario
def protocol_final_line_without_newline(s):
    s.final_newline = False
    s.create("q")
    s.send("q", "x", 1)
    s.list(["q"])


@scenario
def protocol_empty_input(s):
    pass


@scenario
def protocol_ignored_members_and_null(s):
    s.cmd("CreateQueue", OK, QueueName="q", Color="blue", Attributes=None, Body=5)
    s.cmd("ListQueues", ok(QueueNames=["q"]), Prefix=None, Extra={"a": [1, 2]}, QueueName=7)
    s.cmd("CreateQueue", MISSING, QueueName=None)
    s.cmd(
        "CreateQueue",
        OK,
        QueueName="p",
        Attributes={"VisibilityTimeout": None, "DelaySeconds": 5, "DeadLetterQueue": None},
    )
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(DelaySeconds=5), Counts=counts()), QueueName="p")
    s.cmd(
        "SendMessage",
        ok(MessageId="m-1"),
        QueueName="q",
        Body="x",
        DelaySeconds=None,
        GroupId=None,
        DeduplicationId=None,
        Priority="high",
    )
    s.cmd(
        "ReceiveMessage",
        ok(Messages=[msg(1, 1, "x")]),
        QueueName="q",
        MaxMessages=None,
        VisibilityTimeout=None,
    )
    s.cmd("SendMessage", MISSING, QueueName="q", Body=None)
    s.cmd("Tick", MISSING, Seconds=None)
    s.cmd("SetQueueAttributes", MISSING, QueueName="q", Attributes=None)
    s.cmd("DeleteMessage", MISSING, QueueName="q", ReceiptHandle=None)
    s.cmd("Tick", ok(Now=3), Seconds=3, Unit="s")


@scenario(interactive=True)
def protocol_responses_are_flushed_per_line(s):
    s.create("q")
    s.send("q", "one", 1)
    s.raw("not json", INVALID_REQUEST)
    s.receive("q", [msg(1, 1, "one")])
    s.tick(5, 5)
    s.delete("q", 1)
    s.counts("q", 0, 0, 0)


@scenario
def parameter_types(s):
    s.create("q")
    for seconds in ["5.0", "5.5", '"5"', "5e0", "true", "[5]", '{"n": 5}', "-5", "99999999999999999999999999"]:
        s.raw('{"Command": "Tick", "Seconds": %s}' % seconds, INVALID_VALUE)
    s.raw('{"Command": "Tick", "Seconds": 5}', ok(Now=5))
    s.raw('{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": 2.0}', INVALID_VALUE)
    s.raw('{"Command": "ReceiveMessage", "QueueName": "q", "MaxMessages": "2"}', INVALID_VALUE)
    s.raw('{"Command": "ReceiveMessage", "QueueName": "q", "VisibilityTimeout": 1e1}', INVALID_VALUE)
    s.raw(
        '{"Command": "CreateQueue", "QueueName": "p", "Attributes": {"VisibilityTimeout": 30.0}}',
        INVALID_VALUE,
    )
    s.create("p", INVALID_VALUE, VisibilityTimeout="30")
    s.create("p", INVALID_VALUE, Fifo="true")
    s.create("p", INVALID_VALUE, Fifo=1)
    s.create("p", INVALID_VALUE, ContentBasedDeduplication=0)
    s.create("p", INVALID_VALUE, ContentBasedDeduplication="false")
    s.cmd("SendMessage", INVALID_VALUE, QueueName=5, Body="x")
    s.cmd("SendMessage", INVALID_VALUE, QueueName=["q"], Body="x")
    s.cmd("SendMessage", INVALID_VALUE, QueueName="q", Body=5)
    s.cmd("SendMessage", INVALID_VALUE, QueueName="q", Body=True)
    s.cmd("SendMessage", INVALID_VALUE, QueueName="q", Body=["x"])
    s.cmd("SendMessage", INVALID_VALUE, QueueName="q", Body={"text": "x"})
    s.cmd("GetQueueAttributes", INVALID_VALUE, QueueName=3)
    s.cmd("PurgeQueue", INVALID_VALUE, QueueName=False)
    s.cmd("DeleteQueue", INVALID_VALUE, QueueName={})
    s.list(["q"])
    s.counts("q", 0, 0, 0)


# Queues


@scenario
def create_queue_defaults(s):
    s.cmd("CreateQueue", OK, QueueName="orders")
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(), Counts=counts()), QueueName="orders")
    fifo = {
        "Fifo": True,
        "ContentBasedDeduplication": True,
        "VisibilityTimeout": 0,
        "DelaySeconds": 900,
        "RetentionSeconds": 60,
    }
    s.cmd("CreateQueue", OK, QueueName="jobs.fifo", Attributes=fifo)
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(**fifo), Counts=counts()), QueueName="jobs.fifo")
    dead_letter = {"DeadLetterQueue": "orders", "MaxReceiveCount": 5}
    s.cmd("CreateQueue", OK, QueueName="work", Attributes=dead_letter)
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(**dead_letter), Counts=counts()), QueueName="work")
    s.cmd("CreateQueue", OK, QueueName="empty-attributes", Attributes={})
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(), Counts=counts()), QueueName="empty-attributes")
    s.list(["empty-attributes", "jobs.fifo", "orders", "work"])


@scenario
def create_queue_name_syntax(s):
    valid = ["a", "A-b_9", "x" * 80, "-", "_", "0", "fifo", "a-fifo", "afifo"]
    for name in valid:
        s.create(name)
    for name in ["", "x" * 81, "a b", "a.b", "a/b", "caf\u00e9", "a.fif", "a.", " a", "a\n", "a:b", "a*"]:
        s.create(name, NAME_INVALID)
    valid_fifo = ["a.fifo", "x" * 75 + ".fifo", "-.fifo"]
    for name in valid_fifo:
        s.create(name, Fifo=True)
    for name in [".fifo", "x" * 76 + ".fifo", "a.fifo.fifo", "a.FIFO", "a.Fifo", "a.fifo ", "a b.fifo", "a..fifo"]:
        s.create(name, NAME_INVALID, Fifo=True)
    for name in [7, True, ["a"], {"n": "a"}, 1.5]:
        s.cmd("CreateQueue", INVALID_VALUE, QueueName=name)
    s.cmd("CreateQueue", MISSING)
    s.cmd("CreateQueue", MISSING, Attributes={})
    s.list(sorted(valid + valid_fifo))


@scenario
def create_queue_fifo_suffix(s):
    s.create("a.fifo", NAME_INVALID)
    s.create("a.fifo", NAME_INVALID, Fifo=False)
    s.create("a", NAME_INVALID, Fifo=True)
    s.create("afifo", NAME_INVALID, Fifo=True)
    s.create("a-fifo", NAME_INVALID, Fifo=True)
    s.list([])
    s.create("a.fifo", Fifo=True)
    s.create("b", Fifo=False)
    s.create("afifo")
    s.create("c.fifo", Fifo=True, ContentBasedDeduplication=True)
    s.list(["a.fifo", "afifo", "b", "c.fifo"])
    s.counts("a.fifo")
    s.counts("b")
    s.counts("c.fifo")


@scenario
def create_queue_attribute_ranges(s):
    s.create("dlq")
    created = ["dlq"]
    for name, low, high in [
        ("VisibilityTimeout", 0, 43200),
        ("DelaySeconds", 0, 900),
        ("RetentionSeconds", 60, 1209600),
        ("MaxReceiveCount", 1, 1000),
    ]:
        extra = {"DeadLetterQueue": "dlq"} if name == "MaxReceiveCount" else {}
        for value, valid in [(low - 1, False), (low, True), (high, True), (high + 1, False), (-1, False)]:
            queue_name = "%s-%d" % (name, value) if valid else "rejected"
            attributes = dict(extra)
            attributes[name] = value
            s.create(queue_name, None if valid else INVALID_VALUE, **attributes)
            if valid:
                created.append(queue_name)
                s.counts(queue_name)
    s.list(sorted(created))


@scenario
def create_queue_attribute_types(s):
    for attributes in [[], "x", 5, True, [{"VisibilityTimeout": 5}]]:
        s.cmd("CreateQueue", INVALID_VALUE, QueueName="q", Attributes=attributes)
    for attributes in [
        {"Color": "red"},
        {"visibilitytimeout": 5},
        {"VisibilityTimeout": 5, "Unknown": None, "Other": 1},
        {"VisibilityTimeout": True},
        {"VisibilityTimeout": [5]},
        {"DelaySeconds": "0"},
        {"RetentionSeconds": {}},
        {"Fifo": "yes"},
        {"ContentBasedDeduplication": 0},
        {"DeadLetterQueue": 5, "MaxReceiveCount": 3},
        {"DeadLetterQueue": ["d"], "MaxReceiveCount": 3},
        {"DeadLetterQueue": "d", "MaxReceiveCount": "3"},
        {"DeadLetterQueue": "d", "MaxReceiveCount": True},
    ]:
        s.cmd("CreateQueue", INVALID_VALUE, QueueName="q", Attributes=attributes)
    s.list([])


@scenario
def create_queue_twice(s):
    s.create("q", VisibilityTimeout=10)
    s.create("q", VisibilityTimeout=10)
    s.create(
        "q",
        VisibilityTimeout=10,
        DelaySeconds=0,
        RetentionSeconds=345600,
        Fifo=False,
        ContentBasedDeduplication=False,
    )
    s.cmd("CreateQueue", ALREADY_EXISTS, QueueName="q")
    s.cmd("CreateQueue", ALREADY_EXISTS, QueueName="q", Attributes={})
    s.create("q", ALREADY_EXISTS, VisibilityTimeout=11)
    s.create("q", ALREADY_EXISTS, VisibilityTimeout=10, DelaySeconds=1)
    s.send("q", "kept", 1)
    s.create("q", VisibilityTimeout=10)
    s.counts("q", 1, 0, 0)
    s.set("q", VisibilityTimeout=11)
    s.create("q", ALREADY_EXISTS, VisibilityTimeout=10)
    s.create("q", VisibilityTimeout=11)
    s.counts("q", 1, 0, 0)

    s.create("d")
    s.create("d2")
    s.create("w", DeadLetterQueue="d", MaxReceiveCount=3)
    s.create("w", DeadLetterQueue="d", MaxReceiveCount=3)
    s.create("w", ALREADY_EXISTS, DeadLetterQueue="d", MaxReceiveCount=4)
    s.create("w", ALREADY_EXISTS, DeadLetterQueue="d2", MaxReceiveCount=3)
    s.cmd("CreateQueue", ALREADY_EXISTS, QueueName="w")
    s.create("d", ALREADY_EXISTS, DeadLetterQueue="d2", MaxReceiveCount=3)

    s.create("f.fifo", Fifo=True)
    s.create("f.fifo", Fifo=True)
    s.create("f.fifo", Fifo=True, ContentBasedDeduplication=False)
    s.create("f.fifo", ALREADY_EXISTS, Fifo=True, ContentBasedDeduplication=True)
    s.list(["d", "d2", "f.fifo", "q", "w"])


@scenario
def create_queue_dead_letter_rules(s):
    s.create("d")
    s.create("df.fifo", Fifo=True)
    s.create("a", INVALID_VALUE, DeadLetterQueue="d")
    s.create("a", INVALID_VALUE, MaxReceiveCount=3)
    s.create("a", NO_QUEUE, DeadLetterQueue="nope", MaxReceiveCount=3)
    s.create("a", NO_QUEUE, DeadLetterQueue="a", MaxReceiveCount=3)
    s.create("a", NO_QUEUE, DeadLetterQueue="", MaxReceiveCount=3)
    s.create("a", NO_QUEUE, DeadLetterQueue="not a name", MaxReceiveCount=3)
    s.create("a", INVALID_VALUE, DeadLetterQueue="df.fifo", MaxReceiveCount=3)
    s.create("a.fifo", INVALID_VALUE, Fifo=True, DeadLetterQueue="d", MaxReceiveCount=3)
    s.list(["d", "df.fifo"])
    s.create("a", DeadLetterQueue="d", MaxReceiveCount=3)
    s.create("a.fifo", Fifo=True, DeadLetterQueue="df.fifo", MaxReceiveCount=1)
    s.counts("a")
    s.counts("a.fifo")
    s.create("b", INVALID_VALUE, ContentBasedDeduplication=True)
    s.create("b", ContentBasedDeduplication=False)
    s.list(["a", "a.fifo", "b", "d", "df.fifo"])


@scenario
def create_queue_check_order(s):
    s.cmd("CreateQueue", MISSING, Attributes={"VisibilityTimeout": -1})
    s.cmd("CreateQueue", MISSING, Attributes="x")
    s.create("bad name", NAME_INVALID, VisibilityTimeout=-1)
    s.cmd("CreateQueue", NAME_INVALID, QueueName="bad name", Attributes="x")
    s.create("bad name", NAME_INVALID, Color=1)
    s.create("", NAME_INVALID, Fifo="yes")
    s.cmd("CreateQueue", INVALID_VALUE, QueueName=5, Attributes={"VisibilityTimeout": -1})
    s.create("a.fifo", INVALID_VALUE, VisibilityTimeout=43201)
    s.create("a", INVALID_VALUE, Fifo=True, DelaySeconds=901)
    s.create("a.fifo", INVALID_VALUE, Fifo="yes")
    s.create("a.fifo", INVALID_VALUE, Color=1)
    s.create("a.fifo", NAME_INVALID, ContentBasedDeduplication=True)
    s.create("a", NAME_INVALID, Fifo=True, DeadLetterQueue="x")
    s.create("a.fifo", NAME_INVALID, MaxReceiveCount=1)
    s.create("q")
    s.create("q", INVALID_VALUE, ContentBasedDeduplication=True)
    s.create("q", INVALID_VALUE, MaxReceiveCount=2)
    s.create("q", INVALID_VALUE, DeadLetterQueue="nope")
    s.create("q", ALREADY_EXISTS, DeadLetterQueue="nope", MaxReceiveCount=2)
    s.create("f.fifo", Fifo=True)
    s.create("q", ALREADY_EXISTS, DeadLetterQueue="f.fifo", MaxReceiveCount=2)
    s.create("new", NO_QUEUE, DeadLetterQueue="nope", MaxReceiveCount=2)
    s.list(["f.fifo", "q"])


@scenario
def queue_limit(s):
    names = ["q%04d" % index for index in range(MAX_QUEUES)]
    for name in names:
        s.create(name)
    s.create("extra", LIMIT)
    s.create("extra.fifo", LIMIT, Fifo=True)
    s.create("q0500")
    s.create("q0500", ALREADY_EXISTS, VisibilityTimeout=1)
    s.create("extra", NO_QUEUE, DeadLetterQueue="nope", MaxReceiveCount=1)
    s.create("extra", INVALID_VALUE, MaxReceiveCount=1)
    s.create("bad name", NAME_INVALID)
    s.create("extra", LIMIT, DeadLetterQueue="q0001", MaxReceiveCount=1)
    s.list(names)
    s.list(names[900:], Prefix="q09")
    s.drop("q0000")
    s.create("extra")
    s.create("another", LIMIT)
    s.list(["extra"] + names[1:])


@scenario
def delete_queue(s):
    s.create("q")
    s.send("q", "x", 1)
    s.receive("q", [msg(1, 1, "x")])
    s.drop("q")
    s.cmd("GetQueueAttributes", NO_QUEUE, QueueName="q")
    s.send("q", "x", expect=NO_QUEUE)
    s.drop("q", NO_QUEUE)
    s.list([])
    s.cmd("DeleteQueue", MISSING)
    s.create("q", VisibilityTimeout=5)
    s.counts("q", 0, 0, 0)
    s.delete("q", 1, HANDLE_EXPIRED)
    s.send("q", "y", 2)
    s.receive("q", [msg(2, 2, "y")])

    s.create("f.fifo", Fifo=True)
    s.fsend("f.fifo", "a", "g", "d1", 3, 1)
    s.fsend("f.fifo", "b", "g", "d2", 4, 2)
    s.drop("f.fifo")
    s.create("f.fifo", Fifo=True)
    s.fsend("f.fifo", "a", "g", "d1", 5, 1)
    s.counts("f.fifo", 1, 0, 0)


@scenario
def delete_queue_clears_dead_letter_configuration(s):
    s.create("d")
    s.create("a", VisibilityTimeout=0, DeadLetterQueue="d", MaxReceiveCount=1)
    s.create("b", DeadLetterQueue="d", MaxReceiveCount=2)
    s.create("c")
    s.set("c", DeadLetterQueue="a", MaxReceiveCount=3)
    s.drop("d")
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(VisibilityTimeout=0), Counts=counts()), QueueName="a")
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(), Counts=counts()), QueueName="b")
    s.cmd(
        "GetQueueAttributes",
        ok(Attributes=attrs(DeadLetterQueue="a", MaxReceiveCount=3), Counts=counts()),
        QueueName="c",
    )
    s.send("a", "x", 1)
    s.receive("a", [msg(1, 1, "x", count=1)])
    s.receive("a", [msg(1, 2, "x", count=2)])
    s.receive("a", [msg(1, 3, "x", count=3)])
    s.create("a", VisibilityTimeout=0)
    s.cmd("CreateQueue", OK, QueueName="b")
    s.create("d")
    s.counts("a", 1, 0, 0)
    s.counts("b")


@scenario
def list_queues(s):
    s.list([])
    s.list([], Prefix="a")
    for name in ["b", "B", "a", "_x", "-x", "0", "aa", "Zz", "a-1", "a_1", "a0"]:
        s.create(name)
    s.create("a.fifo", Fifo=True)
    everything = ["-x", "0", "B", "Zz", "_x", "a", "a-1", "a.fifo", "a0", "a_1", "aa", "b"]
    s.list(everything)
    s.list(everything, Prefix="")
    s.list(["a", "a-1", "a.fifo", "a0", "a_1", "aa"], Prefix="a")
    s.list(["a.fifo"], Prefix="a.")
    s.list(["a.fifo"], Prefix="a.fifo")
    s.list([], Prefix="a.fifo.")
    s.list([], Prefix="zzz")
    s.list([], Prefix="A")
    s.list(["Zz"], Prefix="Z")
    s.list(["B"], Prefix="B")
    s.list([], Prefix=" ")
    s.list([], Prefix="a b")
    s.cmd("ListQueues", INVALID_VALUE, Prefix=5)
    s.cmd("ListQueues", INVALID_VALUE, Prefix=["a"])
    s.cmd("ListQueues", INVALID_VALUE, Prefix=True)
    s.drop("a")
    s.list(["a-1", "a.fifo", "a0", "a_1", "aa"], Prefix="a")


@scenario
def queue_does_not_exist(s):
    s.create("Q")
    for name in ["nope", "q", "", "bad name", "x" * 200, "Q.fifo", "Q "]:
        s.cmd("DeleteQueue", NO_QUEUE, QueueName=name)
        s.cmd("GetQueueAttributes", NO_QUEUE, QueueName=name)
        s.cmd("SetQueueAttributes", NO_QUEUE, QueueName=name, Attributes={"VisibilityTimeout": 1})
        s.cmd("SetQueueAttributes", NO_QUEUE, QueueName=name, Attributes={})
        s.cmd("PurgeQueue", NO_QUEUE, QueueName=name)
        s.cmd("SendMessage", NO_QUEUE, QueueName=name, Body="x")
        s.cmd("ReceiveMessage", NO_QUEUE, QueueName=name)
        s.cmd("DeleteMessage", NO_QUEUE, QueueName=name, ReceiptHandle="r-1")
        s.cmd("ChangeMessageVisibility", NO_QUEUE, QueueName=name, ReceiptHandle="r-1", VisibilityTimeout=1)
    s.send("Q", "x", 1)
    s.list(["Q"])


@scenario
def get_queue_attributes_counts(s):
    s.create("q", DelaySeconds=10, VisibilityTimeout=20)
    s.send("q", "a", 1)
    s.send("q", "b", 2, DelaySeconds=0)
    s.send("q", "c", 3, DelaySeconds=5)
    s.counts("q", 1, 0, 2)
    s.receive("q", [msg(2, 1, "b")])
    s.counts("q", 0, 1, 2)
    s.tick(4, 4)
    s.counts("q", 0, 1, 2)
    s.tick(1, 5)
    s.counts("q", 1, 1, 1)
    s.tick(5, 10)
    s.counts("q", 2, 1, 0)
    s.tick(9, 19)
    s.counts("q", 2, 1, 0)
    s.tick(1, 20)
    s.counts("q", 3, 0, 0)
    s.cmd("GetQueueAttributes", MISSING)


@scenario
def set_queue_attributes(s):
    s.create("q")
    s.set("q", VisibilityTimeout=5)
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(VisibilityTimeout=5), Counts=counts()), QueueName="q")
    s.set("q", DelaySeconds=3, RetentionSeconds=60)
    s.cmd(
        "GetQueueAttributes",
        ok(Attributes=attrs(VisibilityTimeout=5, DelaySeconds=3, RetentionSeconds=60), Counts=counts()),
        QueueName="q",
    )
    s.set("q")
    s.set("q", ContentBasedDeduplication=False)
    s.counts("q")
    s.cmd("SetQueueAttributes", MISSING, QueueName="q")
    s.cmd("SetQueueAttributes", MISSING, Attributes={})
    for attributes in ["x", [], 5, True]:
        s.cmd("SetQueueAttributes", INVALID_VALUE, QueueName="q", Attributes=attributes)
    s.create("d")
    for attributes in [
        {"Color": "red"},
        {"VisibilityTimeout": -1},
        {"VisibilityTimeout": 43201},
        {"VisibilityTimeout": "5"},
        {"DelaySeconds": -1},
        {"DelaySeconds": 901},
        {"RetentionSeconds": 59},
        {"RetentionSeconds": 1209601},
        {"DeadLetterQueue": "d", "MaxReceiveCount": 0},
        {"DeadLetterQueue": "d", "MaxReceiveCount": 1001},
        {"DeadLetterQueue": 5, "MaxReceiveCount": 1},
        {"ContentBasedDeduplication": "no"},
        {"VisibilityTimeout": 1, "DelaySeconds": 901},
    ]:
        s.cmd("SetQueueAttributes", INVALID_VALUE, QueueName="q", Attributes=attributes)
    s.counts("q")
    s.set("q", VisibilityTimeout=0, DelaySeconds=0, RetentionSeconds=1209600)
    s.counts("q")
    s.set("q", VisibilityTimeout=43200, DelaySeconds=900, RetentionSeconds=60)
    s.set("q", DeadLetterQueue="d", MaxReceiveCount=1)
    s.set("q", DeadLetterQueue="d", MaxReceiveCount=1000)
    s.cmd(
        "GetQueueAttributes",
        ok(
            Attributes=attrs(
                VisibilityTimeout=43200,
                DelaySeconds=900,
                RetentionSeconds=60,
                DeadLetterQueue="d",
                MaxReceiveCount=1000,
            ),
            Counts=counts(),
        ),
        QueueName="q",
    )


@scenario
def set_queue_attributes_rules(s):
    s.create("q")
    s.create("d")
    s.create("d2")
    s.create("f.fifo", Fifo=True)
    s.create("df.fifo", Fifo=True)
    s.set("q", UNSUPPORTED, Fifo=False)
    s.set("q", UNSUPPORTED, Fifo=True)
    s.set("f.fifo", UNSUPPORTED, Fifo=True)
    s.set("q", INVALID_VALUE, ContentBasedDeduplication=True)
    s.set("f.fifo", ContentBasedDeduplication=True)
    s.counts("f.fifo")
    s.set("q", INVALID_VALUE, DeadLetterQueue="d")
    s.set("q", INVALID_VALUE, MaxReceiveCount=2)
    s.set("q", NO_QUEUE, DeadLetterQueue="nope", MaxReceiveCount=2)
    s.set("q", INVALID_VALUE, DeadLetterQueue="q", MaxReceiveCount=2)
    s.set("q", INVALID_VALUE, DeadLetterQueue="df.fifo", MaxReceiveCount=2)
    s.set("f.fifo", INVALID_VALUE, DeadLetterQueue="d", MaxReceiveCount=2)
    s.set("f.fifo", INVALID_VALUE, DeadLetterQueue="f.fifo", MaxReceiveCount=2)
    s.counts("q")
    s.set("q", DeadLetterQueue="d", MaxReceiveCount=2)
    s.counts("q")
    s.set("q", DeadLetterQueue="d", MaxReceiveCount=7)
    s.set("q", DeadLetterQueue="d2", MaxReceiveCount=7)
    s.counts("q")
    s.set("q", INVALID_VALUE, MaxReceiveCount=3)
    s.set("q", INVALID_VALUE, DeadLetterQueue="d")
    s.set("q", NO_QUEUE, VisibilityTimeout=1, DeadLetterQueue="nope", MaxReceiveCount=1)
    s.set("q", UNSUPPORTED, VisibilityTimeout=1, Fifo=False)
    s.set("q", INVALID_VALUE, VisibilityTimeout=1, ContentBasedDeduplication=True)
    s.set("q", INVALID_VALUE, VisibilityTimeout=1, DelaySeconds=-1)
    s.counts("q")
    s.set("d2", DeadLetterQueue="q", MaxReceiveCount=1)
    s.set("f.fifo", DeadLetterQueue="df.fifo", MaxReceiveCount=4)
    s.counts("d2")
    s.counts("f.fifo")


@scenario
def set_queue_attributes_check_order(s):
    s.create("q")
    s.cmd("SetQueueAttributes", MISSING, Attributes={"VisibilityTimeout": -1})
    s.cmd("SetQueueAttributes", MISSING, QueueName="nope")
    s.cmd("SetQueueAttributes", MISSING, QueueName=5)
    s.set("nope", INVALID_VALUE, VisibilityTimeout=-1)
    s.set("nope", INVALID_VALUE, Color=1)
    s.set("nope", INVALID_VALUE, Fifo="x")
    s.cmd("SetQueueAttributes", INVALID_VALUE, QueueName="nope", Attributes=[])
    s.set("nope", NO_QUEUE, Fifo=True)
    s.set("nope", NO_QUEUE, DeadLetterQueue="d")
    s.set("nope", NO_QUEUE, ContentBasedDeduplication=True)
    s.set("q", UNSUPPORTED, Fifo=True, ContentBasedDeduplication=True)
    s.set("q", UNSUPPORTED, Fifo=True, DeadLetterQueue="d")
    s.set("q", UNSUPPORTED, Fifo=False, DeadLetterQueue="nope", MaxReceiveCount=1)
    s.set("q", INVALID_VALUE, VisibilityTimeout=43201, Fifo=True)
    s.set("q", INVALID_VALUE, DeadLetterQueue="nope")
    s.set("q", INVALID_VALUE, ContentBasedDeduplication=True, DeadLetterQueue="nope", MaxReceiveCount=1)
    s.set("q", NO_QUEUE, DeadLetterQueue="nope", MaxReceiveCount=1)
    s.counts("q")


@scenario
def attribute_changes_are_not_retroactive(s):
    s.create("q", VisibilityTimeout=10, DelaySeconds=5)
    s.send("q", "a", 1)
    s.set("q", DelaySeconds=0)
    s.send("q", "b", 2)
    s.counts("q", 1, 0, 1)
    s.receive("q", [msg(2, 1, "b")], MaxMessages=10)
    s.set("q", VisibilityTimeout=100)
    s.tick(5, 5)
    s.counts("q", 1, 1, 0)
    s.receive("q", [msg(1, 2, "a", first=5)], MaxMessages=10)
    s.tick(5, 10)
    s.receive("q", [msg(2, 3, "b", count=2)], MaxMessages=10)
    s.tick(94, 104)
    s.counts("q", 0, 2, 0)
    s.tick(1, 105)
    s.counts("q", 1, 1, 0)
    s.set("q", DelaySeconds=900)
    s.counts("q", 1, 1, 0)
    s.tick(5, 110)
    s.counts("q", 2, 0, 0)


@scenario
def purge_queue(s):
    s.create("q")
    s.send("q", "a", 1)
    s.send("q", "b", 2)
    s.send("q", "c", 3, DelaySeconds=50)
    s.receive("q", [msg(1, 1, "a")])
    s.counts("q", 1, 1, 1)
    s.purge("q")
    s.counts("q", 0, 0, 0)
    s.delete("q", 1, HANDLE_EXPIRED)
    s.change("q", 1, 5, HANDLE_EXPIRED)
    s.receive("q", [], MaxMessages=10)
    s.tick(60, 60)
    s.receive("q", [], MaxMessages=10)
    s.send("q", "d", 4)
    s.receive("q", [msg(4, 2, "d", sent=60, first=60)])
    s.purge("q")
    s.purge("q")
    s.purge("nope", NO_QUEUE)
    s.cmd("PurgeQueue", MISSING)

    s.create("f.fifo", Fifo=True)
    s.fsend("f.fifo", "x", "g", "d1", 5, 1)
    s.purge("f.fifo")
    s.fsend("f.fifo", "x", "g", "d1", 5, 1)
    s.counts("f.fifo", 0, 0, 0)
    s.fsend("f.fifo", "y", "g", "d2", 6, 2)
    s.receive("f.fifo", [msg(6, 3, "y", sent=60, first=60, group="g", seq=2)])


# Sending


@scenario
def send_message_ids(s):
    s.create("a")
    s.create("b")
    s.send("a", "one", 1)
    s.send("b", "two", 2)
    s.send("a", "", expect=INVALID_VALUE)
    s.send("nope", "x", expect=NO_QUEUE)
    s.send("a", "x", expect=UNSUPPORTED, GroupId="g")
    s.cmd("SendMessage", MISSING, QueueName="a")
    s.send("a", "three", 3)
    tricky = 'line1\nline2\t"quoted" \\ back/slash \u00e9\u20ac\U00010348 {"json": [1, null]} \u0001'
    s.send("b", tricky, 4)
    s.send("b", " ", 5)
    s.receive("b", [msg(2, 1, "two")])
    s.receive("a", [msg(1, 2, "one"), msg(3, 3, "three")], MaxMessages=10)
    s.receive("b", [msg(4, 4, tricky), msg(5, 5, " ")], MaxMessages=10)


@scenario
def send_message_body_bounds(s):
    s.create("q")
    s.send("q", "", expect=INVALID_VALUE)
    accepted = ["a", "a" * 1024, "\u00e9" * 512, "\u20ac" * 341, "\U00010348" * 256]
    rejected = ["a" * 1025, "\u00e9" * 512 + "a", "\u00e9" * 513, "\u20ac" * 342, "\U00010348" * 257, "a" * 5000]
    for index, body in enumerate(accepted):
        s.send("q", body, index + 1)
    for body in rejected:
        s.send("q", body, expect=INVALID_VALUE)
    s.cmd("SendMessage", MISSING, QueueName="q")
    s.counts("q", 5, 0, 0)
    s.receive("q", [msg(index + 1, index + 1, body) for index, body in enumerate(accepted)], MaxMessages=10)


@scenario
def send_message_delay(s):
    s.create("q", DelaySeconds=10, VisibilityTimeout=43200)
    s.create("p", VisibilityTimeout=43200)
    for delay in [-1, 901, "5", 1.5, True]:
        s.send("p", "x", expect=INVALID_VALUE, DelaySeconds=delay)
    s.send("p", "a", 1, DelaySeconds=900)
    s.send("p", "b", 2, DelaySeconds=0)
    s.send("p", "c", 3, DelaySeconds=1)
    s.send("q", "d", 4)
    s.send("q", "e", 5, DelaySeconds=0)
    s.send("q", "f", 6, DelaySeconds=20)
    s.counts("p", 1, 0, 2)
    s.counts("q", 1, 0, 2)
    s.receive("p", [msg(2, 1, "b")], MaxMessages=10)
    s.receive("q", [msg(5, 2, "e")], MaxMessages=10)
    s.tick(1, 1)
    s.receive("p", [msg(3, 3, "c", first=1)], MaxMessages=10)
    s.tick(8, 9)
    s.receive("q", [], MaxMessages=10)
    s.tick(1, 10)
    s.receive("q", [msg(4, 4, "d", first=10)], MaxMessages=10)
    s.tick(9, 19)
    s.receive("q", [], MaxMessages=10)
    s.tick(1, 20)
    s.receive("q", [msg(6, 5, "f", first=20)], MaxMessages=10)
    s.tick(879, 899)
    s.receive("p", [], MaxMessages=10)
    s.counts("p", 0, 2, 1)
    s.tick(1, 900)
    s.receive("p", [msg(1, 6, "a", first=900)], MaxMessages=10)


@scenario
def send_message_fifo_requirements(s):
    s.create("f.fifo", Fifo=True)
    s.create("s")
    s.send("f.fifo", "a", expect=MISSING, DeduplicationId="d")
    s.send("f.fifo", "a", expect=MISSING, GroupId="g")
    s.send("f.fifo", "a", expect=MISSING)
    s.fsend("f.fifo", "a", "g", "d1", 1, 1)
    s.fsend("f.fifo", "b", "g", "d2", 2, 2)
    s.send("f.fifo", "c", expect=UNSUPPORTED, GroupId="g2", DeduplicationId="d3", DelaySeconds=0)
    s.send("s", "a", expect=UNSUPPORTED, GroupId="g")
    s.send("s", "a", expect=UNSUPPORTED, DeduplicationId="d")
    s.send("s", "a", expect=UNSUPPORTED, GroupId="g", DeduplicationId="d")
    s.send("s", "a", 3)
    for group in ["", "x" * 129, "a b", "g.1", "caf\u00e9", 5, ["g"], True]:
        s.send("f.fifo", "x", expect=INVALID_VALUE, GroupId=group, DeduplicationId="d9")
        s.send("s", "x", expect=INVALID_VALUE, GroupId=group)
    for dedup in ["", "x" * 129, "d!", "d d", 5, {"id": 1}]:
        s.send("f.fifo", "x", expect=INVALID_VALUE, GroupId="g", DeduplicationId=dedup)
        s.send("s", "x", expect=INVALID_VALUE, DeduplicationId=dedup)
    s.fsend("f.fifo", "c", "x" * 128, "d4", 4, 3)
    s.fsend("f.fifo", "d", "A-b_9", "y" * 128, 5, 4)
    s.counts("f.fifo", 4, 0, 0)
    s.counts("s", 1, 0, 0)


@scenario
def send_message_check_order(s):
    s.create("s")
    s.create("f.fifo", Fifo=True)
    s.cmd("SendMessage", MISSING)
    s.cmd("SendMessage", MISSING, QueueName="nope")
    s.cmd("SendMessage", MISSING, Body="x")
    s.cmd("SendMessage", MISSING, QueueName="nope", DelaySeconds=901)
    s.cmd("SendMessage", MISSING, QueueName=5)
    s.send("nope", "", expect=INVALID_VALUE)
    s.send("nope", "x", expect=INVALID_VALUE, DelaySeconds=901)
    s.send("nope", "x", expect=INVALID_VALUE, GroupId="")
    s.send("nope", "x", expect=NO_QUEUE, GroupId="g")
    s.send("nope", "x", expect=NO_QUEUE, DelaySeconds=5)
    s.send("s", "x", expect=INVALID_VALUE, GroupId="bad id!")
    s.send("s", "", expect=INVALID_VALUE, GroupId="g")
    s.send("f.fifo", "x", expect=INVALID_VALUE, DelaySeconds=901)
    s.send("f.fifo", "x", expect=UNSUPPORTED, DelaySeconds=5)
    s.send("f.fifo", "x", expect=UNSUPPORTED, DelaySeconds=5, GroupId="g")
    s.send("f.fifo", "x", expect=MISSING)
    s.fsend("f.fifo", "x", "g", "d", 1, 1)
    s.send("f.fifo", "x", expect=UNSUPPORTED, GroupId="g", DeduplicationId="d", DelaySeconds=1)
    s.send("f.fifo", "x", expect=MISSING, DeduplicationId="d")
    s.fsend("f.fifo", "x", "g", "d", 1, 1)
    s.counts("f.fifo", 1, 0, 0)
    s.counts("s", 0, 0, 0)
    s.send("s", "x", 2)


@scenario
def message_limit_standard(s):
    s.create("q")
    s.create("other")
    for index in range(MAX_MESSAGES):
        s.send("q", "b%d" % index, index + 1)
    s.send("q", "one too many", expect=LIMIT)
    s.send("q", "", expect=INVALID_VALUE)
    s.send("q", "x", expect=UNSUPPORTED, GroupId="g")
    s.counts("q", MAX_MESSAGES, 0, 0)
    s.send("other", "fine", MAX_MESSAGES + 1)
    s.receive("q", [msg(1, 1, "b0")])
    s.send("q", "still full", expect=LIMIT)
    s.delete("q", 1)
    s.send("q", "fits again", MAX_MESSAGES + 2)
    s.send("q", "full again", expect=LIMIT)
    s.counts("q", MAX_MESSAGES, 0, 0)


@scenario
def message_limit_fifo(s):
    s.create("f.fifo", Fifo=True, ContentBasedDeduplication=True)
    for index in range(MAX_MESSAGES):
        s.send("f.fifo", "b%d" % index, index + 1, index + 1, GroupId="g%d" % (index % 7))
    s.send("f.fifo", "one too many", expect=LIMIT, GroupId="g0")
    s.send("f.fifo", "b5", 6, 6, GroupId="g0")
    s.send("f.fifo", "no group", expect=MISSING)
    s.send("f.fifo", "delayed", expect=UNSUPPORTED, GroupId="g0", DelaySeconds=1)
    s.counts("f.fifo", MAX_MESSAGES, 0, 0)
    s.purge("f.fifo")
    s.send("f.fifo", "after purge", MAX_MESSAGES + 1, MAX_MESSAGES + 1, GroupId="g0")
    s.counts("f.fifo", 1, 0, 0)


@scenario
def deduplication_window(s):
    s.create("f.fifo", Fifo=True)
    s.create("o.fifo", Fifo=True)
    s.fsend("f.fifo", "a", "g", "d1", 1, 1)
    s.fsend("f.fifo", "b", "g", "d1", 1, 1)
    s.fsend("f.fifo", "c", "g2", "d1", 1, 1)
    s.fsend("o.fifo", "a", "g", "d1", 2, 1)
    s.fsend("f.fifo", "d", "g", "d2", 3, 2)
    s.fsend("f.fifo", "d", "g", "D1", 4, 3)
    s.counts("f.fifo", 3, 0, 0)
    s.tick(299, 299)
    s.fsend("f.fifo", "e", "g", "d1", 1, 1)
    s.tick(1, 300)
    s.fsend("f.fifo", "e", "g", "d1", 5, 4)
    s.fsend("f.fifo", "e2", "g", "d1", 5, 4)
    s.fsend("f.fifo", "f", "g", "d2", 6, 5)
    s.tick(299, 599)
    s.fsend("f.fifo", "g", "g", "d1", 5, 4)
    s.tick(1, 600)
    s.fsend("f.fifo", "g", "g", "d1", 7, 6)
    s.counts("f.fifo", 6, 0, 0)
    s.counts("o.fifo", 1, 0, 0)


@scenario
def deduplication_outlives_the_message(s):
    s.create("f.fifo", Fifo=True)
    s.fsend("f.fifo", "a", "g", "d1", 1, 1)
    s.receive("f.fifo", [msg(1, 1, "a", group="g", seq=1)])
    s.delete("f.fifo", 1)
    s.fsend("f.fifo", "a", "g", "d1", 1, 1)
    s.counts("f.fifo", 0, 0, 0)
    s.receive("f.fifo", [])
    s.tick(299, 299)
    s.fsend("f.fifo", "a", "g", "d1", 1, 1)
    s.tick(1, 300)
    s.fsend("f.fifo", "a", "g", "d1", 2, 2)
    s.receive("f.fifo", [msg(2, 2, "a", sent=300, first=300, group="g", seq=2)])


@scenario
def deduplication_content_based(s):
    s.create("c.fifo", Fifo=True, ContentBasedDeduplication=True)
    s.send("c.fifo", "hello", 1, 1, GroupId="g")
    s.send("c.fifo", "hello", 1, 1, GroupId="g")
    s.send("c.fifo", "hello", 1, 1, GroupId="g2")
    s.send("c.fifo", "world", 2, 2, GroupId="g")
    s.send("c.fifo", "Hello", 3, 3, GroupId="g")
    s.fsend("c.fifo", "hello", "g", "x1", 4, 4)
    s.fsend("c.fifo", "other", "g", "x1", 4, 4)
    s.send("c.fifo", "x1", 4, 4, GroupId="g")
    s.fsend("c.fifo", "fresh", "g", "hello", 1, 1)
    long_body = "a b! " * 160
    s.send("c.fifo", long_body, 5, 5, GroupId="g")
    s.send("c.fifo", long_body, 5, 5, GroupId="g")
    s.counts("c.fifo", 5, 0, 0)
    s.tick(300, 300)
    s.send("c.fifo", "hello", 6, 6, GroupId="g")
    s.set("c.fifo", ContentBasedDeduplication=False)
    s.send("c.fifo", "zzz", expect=MISSING, GroupId="g")
    s.fsend("c.fifo", "zzz", "g", "z", 7, 7)
    s.set("c.fifo", ContentBasedDeduplication=True)
    s.send("c.fifo", "zzz", 8, 8, GroupId="g")
    s.send("c.fifo", "z", 7, 7, GroupId="g")
    s.counts("c.fifo", 8, 0, 0)


# Receiving


@scenario
def receive_message_fields(s):
    s.create("q")
    s.receive("q", [])
    s.send("q", "a", 1)
    s.tick(3, 3)
    s.send("q", "b", 2)
    s.tick(2, 5)
    s.send("q", "c", 3)
    s.receive("q", [msg(1, 1, "a", sent=0, first=5)])
    s.receive("q", [msg(2, 2, "b", sent=3, first=5), msg(3, 3, "c", sent=5, first=5)], MaxMessages=10)
    s.receive("q", [], MaxMessages=10)
    s.tick(30, 35)
    s.receive(
        "q",
        [msg(1, 4, "a", count=2, sent=0, first=5), msg(2, 5, "b", count=2, sent=3, first=5)],
        MaxMessages=2,
    )
    s.receive("q", [msg(3, 6, "c", count=2, sent=5, first=5)], MaxMessages=10)
    s.tick(30, 65)
    everything = [("a", 0), ("b", 3), ("c", 5)]
    s.receive(
        "q",
        [msg(i + 1, 7 + i, body, count=3, sent=sent, first=5) for i, (body, sent) in enumerate(everything)],
        MaxMessages=10,
        VisibilityTimeout=0,
    )
    s.receive(
        "q",
        [msg(i + 1, 10 + i, body, count=4, sent=sent, first=5) for i, (body, sent) in enumerate(everything)],
        MaxMessages=10,
    )
    s.counts("q", 0, 3, 0)


@scenario
def receive_message_max_messages(s):
    s.create("q")
    for index in range(12):
        s.send("q", "b%d" % index, index + 1)
    for value in [0, 11, -1, "3", 2.5, True, [1]]:
        s.receive("q", expect=INVALID_VALUE, MaxMessages=value)
    s.receive("q", [msg(1, 1, "b0")])
    s.receive("q", [msg(2, 2, "b1")], MaxMessages=1)
    s.receive("q", [msg(index + 1, index + 1, "b%d" % index) for index in range(2, 12)], MaxMessages=10)
    s.receive("q", [], MaxMessages=10)
    for index in range(12, 15):
        s.send("q", "b%d" % index, index + 1)
    s.receive("q", [msg(index + 1, index + 1, "b%d" % index) for index in range(12, 15)], MaxMessages=10)
    s.cmd("ReceiveMessage", MISSING)
    s.cmd("ReceiveMessage", MISSING, MaxMessages=0)
    s.counts("q", 0, 15, 0)


@scenario
def receive_message_visibility_override(s):
    s.create("q", VisibilityTimeout=100)
    s.send("q", "a", 1)
    s.send("q", "b", 2)
    s.send("q", "c", 3)
    for value in [-1, 43201, "5", 1.5, False]:
        s.receive("q", expect=INVALID_VALUE, VisibilityTimeout=value)
    s.receive("q", [msg(1, 1, "a")], VisibilityTimeout=5)
    s.receive("q", [msg(2, 2, "b")], VisibilityTimeout=43200)
    s.receive("q", [msg(3, 3, "c")])
    s.tick(4, 4)
    s.receive("q", [])
    s.tick(1, 5)
    s.receive("q", [msg(1, 4, "a", count=2)], VisibilityTimeout=0)
    s.delete("q", 4, HANDLE_EXPIRED)
    s.change("q", 4, 10, HANDLE_EXPIRED)
    s.receive("q", [msg(1, 5, "a", count=3)], VisibilityTimeout=0)
    s.counts("q", 1, 2, 0)
    s.tick(95, 100)
    s.counts("q", 2, 1, 0)
    s.tick(43099, 43199)
    s.counts("q", 2, 1, 0)
    s.tick(1, 43200)
    s.counts("q", 3, 0, 0)


@scenario
def visibility_timeout_boundary(s):
    s.create("q")
    s.send("q", "a", 1)
    s.receive("q", [msg(1, 1, "a")])
    s.tick(29, 29)
    s.receive("q", [])
    s.counts("q", 0, 1, 0)
    s.tick(1, 30)
    s.counts("q", 1, 0, 0)
    s.delete("q", 1, HANDLE_EXPIRED)
    s.change("q", 1, 30, HANDLE_EXPIRED)
    s.counts("q", 1, 0, 0)
    s.receive("q", [msg(1, 2, "a", count=2)])
    s.delete("q", 1, HANDLE_EXPIRED)
    s.delete("q", 2)
    s.counts("q", 0, 0, 0)

    s.create("zero", VisibilityTimeout=0)
    s.send("zero", "z", 2, DelaySeconds=0)
    s.receive("zero", [msg(2, 3, "z", sent=30, first=30)], MaxMessages=10)
    s.counts("zero", 1, 0, 0)
    s.delete("zero", 3, HANDLE_EXPIRED)
    s.receive("zero", [msg(2, 4, "z", count=2, sent=30, first=30)], VisibilityTimeout=1)
    s.delete("zero", 4)
    s.counts("zero", 0, 0, 0)


@scenario
def receive_message_keeps_arrival_order(s):
    s.create("q", VisibilityTimeout=10)
    s.send("q", "a", 1)
    s.send("q", "b", 2)
    s.send("q", "c", 3)
    s.receive("q", [msg(1, 1, "a")])
    s.receive("q", [msg(2, 2, "b")])
    s.tick(10, 10)
    s.receive(
        "q",
        [msg(1, 3, "a", count=2), msg(2, 4, "b", count=2), msg(3, 5, "c", first=10)],
        MaxMessages=10,
    )
    s.change("q", 4, 0)
    s.send("q", "d", 4)
    s.receive("q", [msg(2, 6, "b", count=3), msg(4, 7, "d", sent=10, first=10)], MaxMessages=10)

    s.create("p")
    s.send("p", "x", 5, DelaySeconds=5)
    s.send("p", "y", 6)
    s.send("p", "z", 7, DelaySeconds=2)
    s.tick(5, 15)
    s.receive(
        "p",
        [
            msg(5, 8, "x", sent=10, first=15),
            msg(6, 9, "y", sent=10, first=15),
            msg(7, 10, "z", sent=10, first=15),
        ],
        MaxMessages=10,
    )


@scenario
def fifo_group_blocking(s):
    s.create("f.fifo", Fifo=True)
    s.fsend("f.fifo", "a1", "A", "1", 1, 1)
    s.fsend("f.fifo", "a2", "A", "2", 2, 2)
    s.fsend("f.fifo", "b1", "B", "3", 3, 3)
    s.fsend("f.fifo", "a3", "A", "4", 4, 4)
    s.fsend("f.fifo", "b2", "B", "5", 5, 5)
    s.receive(
        "f.fifo",
        [msg(1, 1, "a1", group="A", seq=1), msg(3, 2, "b1", group="B", seq=3)],
        MaxMessages=10,
    )
    s.receive("f.fifo", [], MaxMessages=10)
    s.counts("f.fifo", 3, 2, 0)
    s.delete("f.fifo", 1)
    s.receive("f.fifo", [msg(2, 3, "a2", group="A", seq=2)], MaxMessages=10)
    s.receive("f.fifo", [], MaxMessages=10)
    s.tick(30, 30)
    s.receive(
        "f.fifo",
        [msg(2, 4, "a2", count=2, group="A", seq=2), msg(3, 5, "b1", count=2, group="B", seq=3)],
        MaxMessages=10,
    )
    s.delete("f.fifo", 5)
    s.delete("f.fifo", 4)
    s.receive("f.fifo", [msg(4, 6, "a3", first=30, group="A", seq=4)])
    s.receive("f.fifo", [msg(5, 7, "b2", first=30, group="B", seq=5)])
    s.receive("f.fifo", [], MaxMessages=10)
    s.counts("f.fifo", 0, 2, 0)


@scenario
def fifo_group_order_across_timeouts(s):
    s.create("f.fifo", Fifo=True, VisibilityTimeout=10)
    s.fsend("f.fifo", "a1", "A", "1", 1, 1)
    s.fsend("f.fifo", "a2", "A", "2", 2, 2)
    s.receive("f.fifo", [msg(1, 1, "a1", group="A", seq=1)], VisibilityTimeout=5)
    s.receive("f.fifo", [])
    s.tick(5, 5)
    s.receive("f.fifo", [msg(1, 2, "a1", count=2, group="A", seq=1)], MaxMessages=10)
    s.change("f.fifo", 2, 0)
    s.counts("f.fifo", 2, 0, 0)
    s.receive("f.fifo", [msg(1, 3, "a1", count=3, group="A", seq=1)], MaxMessages=10)
    s.delete("f.fifo", 3)
    s.fsend("f.fifo", "a3", "A", "3", 3, 3)
    s.receive(
        "f.fifo",
        [msg(2, 4, "a2", sent=0, first=5, group="A", seq=2)],
        MaxMessages=10,
        VisibilityTimeout=0,
    )
    s.receive(
        "f.fifo",
        [msg(2, 5, "a2", count=2, sent=0, first=5, group="A", seq=2)],
        MaxMessages=10,
        VisibilityTimeout=0,
    )
    s.counts("f.fifo", 2, 0, 0)
    s.receive("f.fifo", [msg(2, 6, "a2", count=3, sent=0, first=5, group="A", seq=2)], MaxMessages=10)
    s.delete("f.fifo", 6)
    s.receive("f.fifo", [msg(3, 7, "a3", sent=5, first=5, group="A", seq=3)], MaxMessages=10)


@scenario
def fifo_delayed_message_blocks_its_group(s):
    s.create("f.fifo", Fifo=True, DelaySeconds=10)
    s.fsend("f.fifo", "a1", "A", "1", 1, 1)
    s.set("f.fifo", DelaySeconds=0)
    s.fsend("f.fifo", "a2", "A", "2", 2, 2)
    s.fsend("f.fifo", "b1", "B", "3", 3, 3)
    s.counts("f.fifo", 2, 0, 1)
    s.receive("f.fifo", [msg(3, 1, "b1", group="B", seq=3)], MaxMessages=10)
    s.tick(9, 9)
    s.receive("f.fifo", [], MaxMessages=10)
    s.tick(1, 10)
    s.receive("f.fifo", [msg(1, 2, "a1", first=10, group="A", seq=1)], MaxMessages=10)
    s.delete("f.fifo", 2)
    s.receive("f.fifo", [msg(2, 3, "a2", first=10, group="A", seq=2)], MaxMessages=10)


# Receipt handles


@scenario
def delete_message(s):
    s.create("q")
    s.send("q", "a", 1)
    s.send("q", "b", 2)
    s.receive("q", [msg(1, 1, "a"), msg(2, 2, "b")], MaxMessages=2)
    s.delete("q", 1)
    s.delete("q", 1, HANDLE_EXPIRED)
    s.counts("q", 0, 1, 0)
    s.tick(30, 30)
    s.receive("q", [msg(2, 3, "b", count=2)])
    s.delete("q", 2, HANDLE_EXPIRED)
    s.counts("q", 0, 1, 0)
    s.delete("q", 3)
    s.counts("q", 0, 0, 0)
    s.cmd("DeleteMessage", MISSING, QueueName="q")
    s.cmd("DeleteMessage", MISSING, ReceiptHandle="r-1")
    for handle in [5, True, ["r-1"], {"n": 1}]:
        s.cmd("DeleteMessage", INVALID_VALUE, QueueName="q", ReceiptHandle=handle)
    s.send("q", "c", 3)
    s.receive("q", [msg(3, 4, "c", sent=30, first=30)])


@scenario
def receipt_handle_invalid_or_expired(s):
    s.create("a")
    s.create("b")
    s.send("a", "x", 1)
    s.send("b", "y", 2)
    s.delete("a", "r-1", HANDLE_INVALID)
    s.change("a", "r-1", 5, HANDLE_INVALID)
    s.receive("a", [msg(1, 1, "x")])
    never_issued = [
        "",
        "r-",
        "r-0",
        "r-2",
        "r-01",
        "R-1",
        "r-1 ",
        " r-1",
        "r1",
        "1",
        "m-1",
        "r--1",
        "r-+1",
        "r-1.0",
        "r-1e0",
        "bogus",
        "r-99999999999999999999999999",
    ]
    for handle in never_issued:
        s.delete("a", handle, HANDLE_INVALID)
        s.change("a", handle, 5, HANDLE_INVALID)
    s.delete("b", "r-1", HANDLE_EXPIRED)
    s.change("b", "r-1", 5, HANDLE_EXPIRED)
    s.receive("b", [msg(2, 2, "y")])
    s.delete("a", "r-2", HANDLE_EXPIRED)
    s.delete("a", "r-3", HANDLE_INVALID)
    s.delete("b", "r-2")
    s.delete("b", "r-2", HANDLE_EXPIRED)
    s.change("a", "r-1", 60)
    s.delete("a", "r-1")
    s.delete("a", "r-1", HANDLE_EXPIRED)
    s.change("a", "r-1", 5, HANDLE_EXPIRED)
    s.counts("a", 0, 0, 0)
    s.counts("b", 0, 0, 0)


@scenario
def change_message_visibility(s):
    s.create("q")
    s.send("q", "a", 1)
    s.receive("q", [msg(1, 1, "a")])
    s.tick(10, 10)
    s.change("q", 1, 100)
    s.tick(99, 109)
    s.counts("q", 0, 1, 0)
    s.receive("q", [])
    s.tick(1, 110)
    s.counts("q", 1, 0, 0)
    s.change("q", 1, 5, HANDLE_EXPIRED)
    s.receive("q", [msg(1, 2, "a", count=2)])
    s.change("q", 2, 5)
    s.tick(4, 114)
    s.counts("q", 0, 1, 0)
    s.tick(1, 115)
    s.counts("q", 1, 0, 0)
    s.receive("q", [msg(1, 3, "a", count=3)])
    s.change("q", 3, 0)
    s.counts("q", 1, 0, 0)
    s.delete("q", 3, HANDLE_EXPIRED)
    s.receive("q", [msg(1, 4, "a", count=4)])
    for timeout in [-1, 43201, "5", 2.5, True]:
        s.change("q", 4, timeout, INVALID_VALUE)
    s.cmd("ChangeMessageVisibility", MISSING, QueueName="q", ReceiptHandle="r-4")
    s.change("q", 4, 43200)
    s.tick(43199, 43314)
    s.counts("q", 0, 1, 0)
    s.change("q", 4, 1)
    s.tick(1, 43315)
    s.counts("q", 1, 0, 0)


@scenario
def receipt_handle_check_order(s):
    s.create("q")
    s.send("q", "a", 1)
    s.receive("q", [msg(1, 1, "a")])
    s.cmd("ChangeMessageVisibility", MISSING, QueueName="nope")
    s.cmd("ChangeMessageVisibility", MISSING, QueueName="nope", ReceiptHandle="bogus")
    s.cmd("ChangeMessageVisibility", MISSING, QueueName="q", ReceiptHandle="bogus")
    s.cmd("ChangeMessageVisibility", MISSING, QueueName="q", ReceiptHandle=5)
    s.cmd("ChangeMessageVisibility", MISSING, ReceiptHandle="r-1", VisibilityTimeout=-1)
    s.change("nope", "bogus", -1, INVALID_VALUE)
    s.change("nope", "bogus", 5, NO_QUEUE)
    s.change("nope", "r-1", 5, NO_QUEUE)
    s.change("q", "bogus", 99999, INVALID_VALUE)
    s.cmd("ChangeMessageVisibility", INVALID_VALUE, QueueName="q", ReceiptHandle=5, VisibilityTimeout=5)
    s.cmd("ChangeMessageVisibility", INVALID_VALUE, QueueName="nope", ReceiptHandle=5, VisibilityTimeout=5)
    s.change("q", "bogus", 5, HANDLE_INVALID)
    s.cmd("DeleteMessage", MISSING, QueueName="nope")
    s.cmd("DeleteMessage", INVALID_VALUE, QueueName="nope", ReceiptHandle=5)
    s.delete("nope", "bogus", NO_QUEUE)
    s.delete("q", "bogus", HANDLE_INVALID)
    s.delete("q", 1)


# Dead-letter queues


@scenario
def dead_letter_move(s):
    s.create("dlq")
    s.create("q", VisibilityTimeout=10, DeadLetterQueue="dlq", MaxReceiveCount=2)
    s.send("q", "a", 1)
    s.receive("q", [msg(1, 1, "a")])
    s.tick(10, 10)
    s.receive("q", [msg(1, 2, "a", count=2)])
    s.tick(9, 19)
    s.receive("q", [])
    s.counts("q", 0, 1, 0)
    s.counts("dlq", 0, 0, 0)
    s.tick(1, 20)
    s.counts("q", 1, 0, 0)
    s.counts("dlq", 0, 0, 0)
    s.receive("q", [])
    s.counts("q", 0, 0, 0)
    s.counts("dlq", 1, 0, 0)
    s.delete("q", 2, HANDLE_EXPIRED)
    s.delete("dlq", 2, HANDLE_EXPIRED)
    s.tick(5, 25)
    s.receive("dlq", [msg(1, 3, "a", count=1, sent=0, first=25)])
    s.delete("dlq", 3)
    s.counts("dlq", 0, 0, 0)


@scenario
def dead_letter_move_and_max_messages(s):
    s.create("dlq")
    s.create("q", VisibilityTimeout=0, DeadLetterQueue="dlq", MaxReceiveCount=1)
    s.send("q", "a", 1)
    s.send("q", "b", 2)
    s.send("q", "c", 3)
    s.receive("q", [msg(1, 1, "a"), msg(2, 2, "b")], MaxMessages=2)
    s.receive("q", [msg(3, 3, "c")], MaxMessages=2)
    s.counts("dlq", 2, 0, 0)
    s.counts("q", 1, 0, 0)
    s.receive("q", [], MaxMessages=10)
    s.counts("q", 0, 0, 0)
    s.receive("dlq", [msg(1, 4, "a"), msg(2, 5, "b"), msg(3, 6, "c")], MaxMessages=10)

    s.create("dlq2")
    s.create("p", VisibilityTimeout=0, DeadLetterQueue="dlq2", MaxReceiveCount=1)
    s.send("p", "w", 4, DelaySeconds=5)
    s.send("p", "x", 5)
    s.receive("p", [msg(5, 7, "x")], MaxMessages=10)
    s.tick(5, 5)
    s.receive("p", [msg(4, 8, "w", first=5)])
    s.counts("dlq2", 0, 0, 0)
    s.counts("p", 2, 0, 0)
    s.receive("p", [])
    s.counts("p", 0, 0, 0)
    s.receive("dlq2", [msg(4, 9, "w", first=5), msg(5, 10, "x", first=5)], MaxMessages=10)


@scenario
def dead_letter_fifo(s):
    s.create("dead.fifo", Fifo=True)
    s.create("f.fifo", Fifo=True, VisibilityTimeout=0, DeadLetterQueue="dead.fifo", MaxReceiveCount=1)
    s.fsend("f.fifo", "a1", "A", "d1", 1, 1)
    s.fsend("f.fifo", "a2", "A", "d2", 2, 2)
    s.fsend("f.fifo", "b1", "B", "d3", 3, 3)
    s.receive(
        "f.fifo",
        [msg(1, 1, "a1", group="A", seq=1), msg(3, 2, "b1", group="B", seq=3)],
        MaxMessages=10,
    )
    s.receive("f.fifo", [msg(2, 3, "a2", group="A", seq=2)], MaxMessages=10)
    s.counts("f.fifo", 1, 0, 0)
    s.counts("dead.fifo", 2, 0, 0)
    s.fsend("dead.fifo", "z", "A", "dz", 4, 3)
    s.fsend("dead.fifo", "a1", "A", "d1", 5, 4)
    s.receive(
        "dead.fifo",
        [msg(1, 4, "a1", group="A", seq=1), msg(3, 5, "b1", group="B", seq=2)],
        MaxMessages=10,
    )
    s.receive("dead.fifo", [], MaxMessages=10)
    s.fsend("f.fifo", "a1", "A", "d1", 1, 1)
    s.counts("f.fifo", 1, 0, 0)
    s.delete("dead.fifo", 4)
    s.receive("dead.fifo", [msg(4, 6, "z", group="A", seq=3)], MaxMessages=10)


@scenario
def dead_letter_retention(s):
    s.create("dlq", RetentionSeconds=60)
    s.create("q", VisibilityTimeout=0, RetentionSeconds=1000, DeadLetterQueue="dlq", MaxReceiveCount=1)
    s.send("q", "old", 1)
    s.receive("q", [msg(1, 1, "old")])
    s.tick(59, 59)
    s.send("q", "young", 2)
    s.receive("q", [msg(2, 2, "young", sent=59, first=59)], MaxMessages=10)
    s.counts("dlq", 1, 0, 0)
    s.tick(1, 60)
    s.counts("dlq", 0, 0, 0)
    s.receive("q", [])
    s.counts("dlq", 1, 0, 0)
    s.send("q", "third", 3)
    s.receive("q", [msg(3, 3, "third", sent=60, first=60)])
    s.tick(58, 118)
    s.counts("dlq", 1, 0, 0)
    s.tick(1, 119)
    s.counts("dlq", 0, 0, 0)
    s.tick(41, 160)
    s.counts("q", 1, 0, 0)
    s.receive("q", [])
    s.counts("q", 0, 0, 0)
    s.counts("dlq", 0, 0, 0)
    s.receive("dlq", [], MaxMessages=10)


@scenario
def dead_letter_reconfiguration(s):
    s.create("d1")
    s.create("d2")
    s.create("q", VisibilityTimeout=0, DeadLetterQueue="d1", MaxReceiveCount=3)
    s.send("q", "a", 1)
    s.receive("q", [msg(1, 1, "a")])
    s.receive("q", [msg(1, 2, "a", count=2)])
    s.set("q", DeadLetterQueue="d2", MaxReceiveCount=2)
    s.counts("q", 1, 0, 0)
    s.receive("q", [])
    s.counts("d1", 0, 0, 0)
    s.counts("d2", 1, 0, 0)
    s.send("q", "b", 2)
    s.receive("q", [msg(2, 3, "b")])
    s.receive("q", [msg(2, 4, "b", count=2)])
    s.set("q", DeadLetterQueue="d1", MaxReceiveCount=4)
    s.receive("q", [msg(2, 5, "b", count=3)])
    s.receive("q", [msg(2, 6, "b", count=4)])
    s.receive("q", [])
    s.counts("d1", 1, 0, 0)
    s.set("d1", VisibilityTimeout=0, DeadLetterQueue="d2", MaxReceiveCount=1)
    s.receive("d1", [msg(2, 7, "b")])
    s.receive("d1", [])
    s.receive("d2", [msg(1, 8, "a"), msg(2, 9, "b")], MaxMessages=10)


# Time


@scenario
def retention_boundary(s):
    s.create("q", RetentionSeconds=60, VisibilityTimeout=100)
    s.send("q", "a", 1)
    s.tick(10, 10)
    s.send("q", "b", 2, DelaySeconds=900)
    s.tick(10, 20)
    s.send("q", "c", 3)
    s.receive("q", [msg(1, 1, "a", first=20)])
    s.counts("q", 1, 1, 1)
    s.tick(39, 59)
    s.counts("q", 1, 1, 1)
    s.tick(1, 60)
    s.counts("q", 1, 0, 1)
    s.delete("q", 1, HANDLE_EXPIRED)
    s.tick(9, 69)
    s.counts("q", 1, 0, 1)
    s.tick(1, 70)
    s.counts("q", 1, 0, 0)
    s.tick(9, 79)
    s.receive("q", [msg(3, 2, "c", sent=20, first=79)])
    s.tick(1, 80)
    s.counts("q", 0, 0, 0)
    s.delete("q", 2, HANDLE_EXPIRED)
    s.receive("q", [], MaxMessages=10)
    s.tick(1209600, 1209680)
    s.send("q", "late", 4)
    s.counts("q", 1, 0, 0)


@scenario
def retention_change(s):
    s.create("q", RetentionSeconds=1000)
    s.send("q", "a", 1)
    s.tick(100, 100)
    s.send("q", "b", 2)
    s.set("q", RetentionSeconds=101)
    s.counts("q", 2, 0, 0)
    s.set("q", RetentionSeconds=100)
    s.counts("q", 1, 0, 0)
    s.set("q", RetentionSeconds=1209600)
    s.counts("q", 1, 0, 0)
    s.receive("q", [msg(2, 1, "b", sent=100, first=100)], MaxMessages=10)

    s.create("p", RetentionSeconds=60)
    s.send("p", "c", 3)
    s.tick(59, 159)
    s.set("p", RetentionSeconds=120)
    s.tick(1, 160)
    s.counts("p", 1, 0, 0)
    s.tick(59, 219)
    s.counts("p", 1, 0, 0)
    s.tick(1, 220)
    s.counts("p", 0, 0, 0)


@scenario
def tick(s):
    s.tick(0, 0)
    s.cmd("Tick", MISSING)
    for seconds in [-1, 1209601, "1", 1.5, None]:
        s.cmd("Tick", MISSING if seconds is None else INVALID_VALUE, Seconds=seconds)
    s.tick(1209600, 1209600)
    s.tick(1, 1209601)
    s.tick(0, 1209601)
    s.create("q")
    s.send("q", "a", 1)
    s.receive("q", [msg(1, 1, "a", sent=1209601, first=1209601)])


# Stress


class Lcg:
    def __init__(self, seed):
        self.state = seed

    def below(self, bound):
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) % (1 << 64)
        return (self.state >> 33) % bound

    def pick(self, items):
        return items[self.below(len(items))]


class Model:
    """A second, independent implementation of the message rules, used by the stress scenarios."""

    def __init__(self):
        self.now = 0
        self.queues = {}
        self.accepted_messages = 0
        self.issued_handles = 0

    def create(self, name, **attributes):
        self.queues[name] = {"attributes": attrs(**attributes), "messages": [], "arrivals": 0, "accepted": {}}

    def state(self, message):
        if self.now < message["visible_from"]:
            return "Delayed"
        if message["until"] is not None and self.now < message["until"]:
            return "InFlight"
        return "Visible"

    def expire(self, queue):
        retention = queue["attributes"]["RetentionSeconds"]
        queue["messages"] = [m for m in queue["messages"] if self.now < m["sent"] + retention]

    def append(self, queue, message):
        queue["arrivals"] += 1
        message["seq"] = queue["arrivals"]
        queue["messages"].append(message)

    def send(self, name, body, delay=None, group=None, dedup=None):
        queue = self.queues.get(name)
        if queue is None:
            return NO_QUEUE
        attributes = queue["attributes"]
        if attributes["Fifo"]:
            if delay is not None:
                return UNSUPPORTED
            if group is None:
                return MISSING
            if dedup is None:
                if not attributes["ContentBasedDeduplication"]:
                    return MISSING
                dedup = body
            original = queue["accepted"].get(dedup)
            if original is not None and self.now - original[0] < 300:
                return ok(MessageId=original[1], SequenceNumber=original[2])
        elif group is not None or dedup is not None:
            return UNSUPPORTED
        if len(queue["messages"]) >= MAX_MESSAGES:
            return LIMIT
        self.accepted_messages += 1
        message = {
            "id": "m-%d" % self.accepted_messages,
            "body": body,
            "sent": self.now,
            "visible_from": self.now + (attributes["DelaySeconds"] if delay is None else delay),
            "count": 0,
            "first": None,
            "handle": None,
            "until": None,
            "group": group,
        }
        self.append(queue, message)
        if not attributes["Fifo"]:
            return ok(MessageId=message["id"])
        queue["accepted"][dedup] = (self.now, message["id"], message["seq"])
        return ok(MessageId=message["id"], SequenceNumber=message["seq"])

    def receive(self, name, max_messages=None, timeout=None):
        queue = self.queues.get(name)
        if queue is None:
            return NO_QUEUE
        attributes = queue["attributes"]
        timeout = attributes["VisibilityTimeout"] if timeout is None else timeout
        limit = attributes.get("MaxReceiveCount")
        delivered, moved, staying_groups = [], [], set()
        for message in queue["messages"]:
            if len(delivered) == (1 if max_messages is None else max_messages):
                break
            eligible = self.state(message) == "Visible"
            if attributes["Fifo"] and message["group"] in staying_groups:
                eligible = False
            if eligible and limit is not None and message["count"] >= limit:
                moved.append(message)
                continue
            if attributes["Fifo"]:
                staying_groups.add(message["group"])
            if not eligible:
                continue
            self.issued_handles += 1
            message["count"] += 1
            if message["first"] is None:
                message["first"] = self.now
            message["handle"] = "r-%d" % self.issued_handles
            message["until"] = self.now + timeout
            entry = {
                "MessageId": message["id"],
                "ReceiptHandle": message["handle"],
                "Body": message["body"],
                "ReceiveCount": message["count"],
                "SentAt": message["sent"],
                "FirstReceivedAt": message["first"],
            }
            if attributes["Fifo"]:
                entry["GroupId"] = message["group"]
                entry["SequenceNumber"] = message["seq"]
            delivered.append(entry)
        if moved:
            queue["messages"] = [m for m in queue["messages"] if not any(m is other for other in moved)]
            target = self.queues[attributes["DeadLetterQueue"]]
            for message in moved:
                message.update(count=0, first=None, handle=None, until=None, visible_from=self.now)
                self.append(target, message)
            self.expire(target)
        return ok(Messages=delivered)

    def in_flight(self, name, handle):
        queue = self.queues.get(name)
        if queue is None:
            return NO_QUEUE, None, None
        digits = handle[2:]
        issued = (
            handle.startswith("r-")
            and digits.isdigit()
            and digits.isascii()
            and str(int(digits)) == digits
            and 1 <= int(digits) <= self.issued_handles
        )
        if not issued:
            return HANDLE_INVALID, None, None
        for message in queue["messages"]:
            if message["handle"] == handle and self.state(message) == "InFlight":
                return None, queue, message
        return HANDLE_EXPIRED, None, None

    def delete(self, name, handle):
        failure, queue, message = self.in_flight(name, handle)
        if failure:
            return failure
        queue["messages"] = [m for m in queue["messages"] if m is not message]
        return OK

    def change(self, name, handle, timeout):
        failure, _, message = self.in_flight(name, handle)
        if failure:
            return failure
        message["until"] = self.now + timeout
        return OK

    def tick(self, seconds):
        self.now += seconds
        for queue in self.queues.values():
            self.expire(queue)
        return ok(Now=self.now)

    def set(self, name, **attributes):
        queue = self.queues[name]
        queue["attributes"].update(attributes)
        self.expire(queue)
        return OK

    def purge(self, name):
        self.queues[name]["messages"] = []
        return OK

    def get(self, name):
        queue = self.queues[name]
        states = [self.state(message) for message in queue["messages"]]
        return ok(
            Attributes=dict(queue["attributes"]),
            Counts=counts(states.count("Visible"), states.count("InFlight"), states.count("Delayed")),
        )


def stress(s, seed, queues, commands=2000):
    rng = Lcg(seed)
    model = Model()
    for name, attributes in queues:
        s.create(name, **attributes)
        model.create(name, **attributes)
    names = [name for name, _ in queues]
    issued = []
    for index in range(commands - len(queues)):
        name = rng.pick(names)
        fifo = model.queues[name]["attributes"]["Fifo"]
        content_based = model.queues[name]["attributes"]["ContentBasedDeduplication"]
        roll = rng.below(100)
        if roll < 34:
            params = {"QueueName": name, "Body": "body-%d" % index}
            kwargs = {}
            if fifo:
                kwargs["group"] = "g%d" % rng.below(4)
                if rng.below(3) == 0:
                    params["Body"] = "repeat-%d" % rng.below(12)
                if rng.below(3) == 0 or (not content_based and rng.below(20) != 0):
                    kwargs["dedup"] = "id-%d" % rng.below(60)
                if rng.below(25) == 0:
                    del kwargs["group"]
                if rng.below(25) == 0:
                    kwargs["delay"] = rng.below(5)
            else:
                if rng.below(4) == 0:
                    kwargs["delay"] = rng.pick([0, 1, 3, 10, 40])
                if rng.below(25) == 0:
                    kwargs["group"] = "g1"
            for key, member in [("delay", "DelaySeconds"), ("group", "GroupId"), ("dedup", "DeduplicationId")]:
                if key in kwargs:
                    params[member] = kwargs[key]
            s.cmd("SendMessage", model.send(name, params["Body"], **kwargs), **params)
        elif roll < 60:
            params, kwargs = {}, {}
            if rng.below(4) != 0:
                kwargs["max_messages"] = params["MaxMessages"] = 1 + rng.below(10)
            if rng.below(4) == 0:
                kwargs["timeout"] = params["VisibilityTimeout"] = rng.pick([0, 0, 2, 7, 25, 90])
            expect = model.receive(name, **kwargs)
            issued.extend((name, message["ReceiptHandle"]) for message in expect["Messages"])
            s.cmd("ReceiveMessage", expect, QueueName=name, **params)
        elif roll < 86:
            kind = rng.below(20)
            if kind < 14 and issued:
                name, handle = issued[len(issued) - 1 - rng.below(min(len(issued), 12))]
            elif kind < 17 and issued:
                name, handle = rng.pick(issued)
            elif kind < 19 and issued:
                handle = rng.pick(issued)[1]
            else:
                handle = "r-%d" % (model.issued_handles + 1 + rng.below(3))
            if roll < 78:
                s.delete(name, handle, model.delete(name, handle))
            else:
                timeout = rng.pick([0, 1, 6, 20, 75])
                s.change(name, handle, timeout, model.change(name, handle, timeout))
        elif roll < 94:
            seconds = rng.pick([0, 1, 1, 2, 3, 5, 8, 13, 30, 45])
            s.cmd("Tick", model.tick(seconds), Seconds=seconds)
        elif roll < 98:
            s.cmd("GetQueueAttributes", model.get(name), QueueName=name)
        elif roll < 99:
            timeout = rng.pick([0, 5, 15, 30])
            s.cmd(
                "SetQueueAttributes",
                model.set(name, VisibilityTimeout=timeout),
                QueueName=name,
                Attributes={"VisibilityTimeout": timeout},
            )
        else:
            s.cmd("PurgeQueue", model.purge(name), QueueName=name)
    for name in names:
        s.cmd("GetQueueAttributes", model.get(name), QueueName=name)
        s.cmd("ReceiveMessage", model.receive(name, 10), QueueName=name, MaxMessages=10)


@scenario
def stress_mixed_2000_commands(s):
    stress(
        s,
        20261120,
        [
            ("dlq", {"RetentionSeconds": 900}),
            ("std", {"VisibilityTimeout": 20, "RetentionSeconds": 600, "DeadLetterQueue": "dlq", "MaxReceiveCount": 3}),
            ("slow", {"DelaySeconds": 7, "RetentionSeconds": 120, "VisibilityTimeout": 5}),
            ("dead.fifo", {"Fifo": True}),
            (
                "jobs.fifo",
                {
                    "Fifo": True,
                    "ContentBasedDeduplication": True,
                    "VisibilityTimeout": 15,
                    "DeadLetterQueue": "dead.fifo",
                    "MaxReceiveCount": 2,
                },
            ),
        ],
    )


@scenario
def stress_fifo_2000_commands(s):
    stress(
        s,
        77,
        [
            ("dead.fifo", {"Fifo": True, "RetentionSeconds": 400, "VisibilityTimeout": 3}),
            (
                "a.fifo",
                {
                    "Fifo": True,
                    "ContentBasedDeduplication": True,
                    "VisibilityTimeout": 8,
                    "DeadLetterQueue": "dead.fifo",
                    "MaxReceiveCount": 2,
                },
            ),
            ("b.fifo", {"Fifo": True, "DelaySeconds": 4, "RetentionSeconds": 200}),
        ],
    )


# Feature: batches


def entry(entry_id, **members):
    members["Id"] = entry_id
    return members


def batch(successful, failures=()):
    return ok(Successful=list(successful), Failed=list(failures))


def sent(entry_id, message, seq=None):
    result = {"Id": entry_id, "MessageId": "m-%d" % message}
    if seq is not None:
        result["SequenceNumber"] = seq
    return result


@scenario(feature=True)
def batch_send(s):
    s.create("q")
    s.send_batch(
        "q",
        [entry("a", Body="one"), entry("b", Body="two"), entry("c", Body="three", DelaySeconds=5)],
        batch([sent("a", 1), sent("b", 2), sent("c", 3)]),
    )
    s.counts("q", 2, 0, 1)
    s.receive("q", [msg(1, 1, "one"), msg(2, 2, "two")], MaxMessages=10)
    s.send_batch("q", [entry("only", Body="single")], batch([sent("only", 4)]))
    s.send("q", "plain", 5)
    s.send_batch(
        "q",
        [entry("e%d" % index, Body="b%d" % index) for index in range(10)],
        batch([sent("e%d" % index, 6 + index) for index in range(10)]),
    )
    s.send_batch("q", [entry("a", Body="reused id in a new batch")], batch([sent("a", 16)]))
    s.counts("q", 13, 2, 1)


@scenario(feature=True)
def batch_send_partial_failure(s):
    s.create("q")
    s.send_batch(
        "q",
        [
            entry("a", Body="ok1"),
            entry("b", Body=""),
            entry("c"),
            entry("d", Body="ok2"),
            entry("e", Body=5),
            entry("f", Body="x", DelaySeconds=901),
            entry("g", Body="x", GroupId="g"),
            entry("h", Body="x" * 1025),
            entry("i", Body=None),
            entry("j", Body="ok3", DelaySeconds=None, Whatever=1),
        ],
        batch(
            [sent("a", 1), sent("d", 2), sent("j", 3)],
            [
                failed("b", INVALID_VALUE),
                failed("c", MISSING),
                failed("e", INVALID_VALUE),
                failed("f", INVALID_VALUE),
                failed("g", UNSUPPORTED),
                failed("h", INVALID_VALUE),
                failed("i", MISSING),
            ],
        ),
    )
    s.send("q", "single", 4)
    s.send_batch(
        "q",
        [entry("x", Body=""), entry("y")],
        batch([], [failed("x", INVALID_VALUE), failed("y", MISSING)]),
    )
    s.counts("q", 4, 0, 0)


@scenario(feature=True)
def batch_send_request_errors(s):
    s.create("q")
    valid = [entry("e%d" % index, Body="x") for index in range(11)]
    s.cmd("SendMessageBatch", MISSING, QueueName="q")
    s.cmd("SendMessageBatch", MISSING, QueueName="q", Entries=None)
    s.cmd("SendMessageBatch", MISSING, Entries=valid[:1])
    for entries in ["x", {}, 5, True, {"Id": "a", "Body": "x"}]:
        s.send_batch("q", entries, INVALID_VALUE)
    s.cmd("SendMessageBatch", INVALID_VALUE, QueueName=5, Entries=valid[:1])
    s.send_batch("q", [], EMPTY_BATCH)
    s.send_batch("q", valid, TOO_MANY_ENTRIES)
    s.send_batch("q", valid + valid, TOO_MANY_ENTRIES)
    s.counts("q", 0, 0, 0)
    s.send_batch("q", valid[:10], batch([sent("e%d" % index, index + 1) for index in range(10)]))
    s.send("q", "next", 11)


@scenario(feature=True)
def batch_entry_ids(s):
    s.create("q")
    good = entry("good", Body="x")
    for bad in [
        "x",
        5,
        None,
        [],
        ["a"],
        {"Body": "x"},
        {"Id": 5, "Body": "x"},
        {"Id": None, "Body": "x"},
        {"Id": "", "Body": "x"},
        {"Id": "x" * 81, "Body": "x"},
        {"Id": "a b", "Body": "x"},
        {"Id": "a.b", "Body": "x"},
        {"Id": ["a"], "Body": "x"},
    ]:
        s.send_batch("q", [good, bad], INVALID_ENTRY_ID)
        s.delete_batch("q", [bad], INVALID_ENTRY_ID)
    s.send_batch("q", [good, entry("b", Body="x"), entry("good", Body="y")], IDS_NOT_DISTINCT)
    s.delete_batch(
        "q",
        [entry("a", ReceiptHandle="r-1"), entry("a", ReceiptHandle="r-2")],
        IDS_NOT_DISTINCT,
    )
    s.counts("q", 0, 0, 0)
    s.send_batch(
        "q",
        [entry("x" * 80, Body="x"), entry("a", Body="x"), entry("A", Body="x"), entry("A-b_9", Body="x")],
        batch([sent("x" * 80, 1), sent("a", 2), sent("A", 3), sent("A-b_9", 4)]),
    )


@scenario(feature=True)
def batch_check_order(s):
    s.create("q")
    eleven = [entry("e%d" % index, Body="x") for index in range(11)]
    duplicates = [entry("a", Body="x"), entry("a", Body="y")]
    s.cmd("SendMessageBatch", MISSING, Entries=[])
    s.cmd("SendMessageBatch", MISSING, QueueName="nope")
    s.send_batch("nope", "x", INVALID_VALUE)
    s.delete_batch("nope", {}, INVALID_VALUE)
    for entries in [[], eleven, duplicates, ["x"], [entry("a", Body="x")]]:
        s.send_batch("nope", entries, NO_QUEUE)
        s.delete_batch("nope", entries, NO_QUEUE)
    s.send_batch("q", eleven[:9] + duplicates, TOO_MANY_ENTRIES)
    s.send_batch("q", eleven[:10] + ["x"], TOO_MANY_ENTRIES)
    s.send_batch("q", [entry("bad id", Body="x")] + duplicates, INVALID_ENTRY_ID)
    s.send_batch("q", duplicates + [entry("", Body="x")], INVALID_ENTRY_ID)
    s.send_batch("q", [entry("a", Body=""), entry("a")], IDS_NOT_DISTINCT)
    s.delete_batch("q", eleven[:9] + duplicates, TOO_MANY_ENTRIES)
    s.delete_batch("q", [{"ReceiptHandle": "r-1"}] + duplicates, INVALID_ENTRY_ID)
    s.counts("q", 0, 0, 0)
    s.send("q", "first", 1)


@scenario(feature=True)
def batch_send_fifo(s):
    s.create("f.fifo", Fifo=True)
    s.send_batch(
        "f.fifo",
        [
            entry("a", Body="x", GroupId="g", DeduplicationId="d1"),
            entry("b", Body="y", GroupId="g", DeduplicationId="d1"),
            entry("c", Body="z", GroupId="g", DeduplicationId="d2"),
            entry("d", Body="w", DeduplicationId="d3"),
            entry("e", Body="v", GroupId="g"),
            entry("f", Body="u", GroupId="g", DeduplicationId="d4", DelaySeconds=1),
            entry("g", Body="t", GroupId="bad group", DeduplicationId="d5"),
            entry("h", Body="s", GroupId="h", DeduplicationId="d6"),
        ],
        batch(
            [sent("a", 1, 1), sent("b", 1, 1), sent("c", 2, 2), sent("h", 3, 3)],
            [
                failed("d", MISSING),
                failed("e", MISSING),
                failed("f", UNSUPPORTED),
                failed("g", INVALID_VALUE),
            ],
        ),
    )
    s.counts("f.fifo", 3, 0, 0)
    s.fsend("f.fifo", "again", "g", "d2", 2, 2)
    s.receive(
        "f.fifo",
        [msg(1, 1, "x", group="g", seq=1), msg(3, 2, "s", group="h", seq=3)],
        MaxMessages=10,
    )


@scenario(feature=True)
def batch_send_message_limit(s):
    s.create("q")
    message = 0
    for _ in range(MAX_MESSAGES // 10 - 1):
        s.send_batch(
            "q",
            [entry("e%d" % index, Body="x") for index in range(10)],
            batch([sent("e%d" % index, message + index + 1) for index in range(10)]),
        )
        message += 10
    s.send_batch(
        "q",
        [entry("e%d" % index, Body="x") for index in range(5)],
        batch([sent("e%d" % index, message + index + 1) for index in range(5)]),
    )
    message += 5
    s.counts("q", MAX_MESSAGES - 5, 0, 0)
    s.send_batch(
        "q",
        [entry("e%d" % index, Body="x") for index in range(10)],
        batch(
            [sent("e%d" % index, message + index + 1) for index in range(5)],
            [failed("e%d" % index, LIMIT) for index in range(5, 10)],
        ),
    )
    s.counts("q", MAX_MESSAGES, 0, 0)
    s.send("q", "x", expect=LIMIT)


@scenario(feature=True)
def batch_delete(s):
    s.create("q")
    s.send("q", "a", 1)
    s.send("q", "b", 2)
    s.send("q", "c", 3)
    s.receive("q", [msg(1, 1, "a"), msg(2, 2, "b"), msg(3, 3, "c")], MaxMessages=10)
    s.delete_batch(
        "q",
        [entry("x", ReceiptHandle="r-1"), entry("y", ReceiptHandle="r-3")],
        batch([{"Id": "x"}, {"Id": "y"}]),
    )
    s.counts("q", 0, 1, 0)
    s.delete_batch("q", [entry("z", ReceiptHandle="r-2", Body="ignored")], batch([{"Id": "z"}]))
    s.counts("q", 0, 0, 0)


@scenario(feature=True)
def batch_delete_partial_failure(s):
    s.create("q")
    s.create("p")
    s.send("q", "a", 1)
    s.send("q", "b", 2)
    s.send("p", "c", 3)
    s.receive("q", [msg(1, 1, "a"), msg(2, 2, "b")], MaxMessages=10)
    s.receive("p", [msg(3, 3, "c")])
    s.delete_batch(
        "q",
        [
            entry("a", ReceiptHandle="r-1"),
            entry("b", ReceiptHandle="r-1"),
            entry("c", ReceiptHandle="bogus"),
            entry("d", ReceiptHandle="r-3"),
            entry("e"),
            entry("f", ReceiptHandle=5),
            entry("g", ReceiptHandle="r-2"),
            entry("h", ReceiptHandle="r-9"),
            entry("i", ReceiptHandle=None),
            entry("j", ReceiptHandle="r-02"),
        ],
        batch(
            [{"Id": "a"}, {"Id": "g"}],
            [
                failed("b", HANDLE_EXPIRED),
                failed("c", HANDLE_INVALID),
                failed("d", HANDLE_EXPIRED),
                failed("e", MISSING),
                failed("f", INVALID_VALUE),
                failed("h", HANDLE_INVALID),
                failed("i", MISSING),
                failed("j", HANDLE_INVALID),
            ],
        ),
    )
    s.counts("q", 0, 0, 0)
    s.counts("p", 0, 1, 0)


@scenario(feature=True)
def batch_delete_request_errors(s):
    s.create("q")
    s.send("q", "a", 1)
    s.receive("q", [msg(1, 1, "a")])
    eleven = [entry("e%d" % index, ReceiptHandle="r-1") for index in range(11)]
    s.cmd("DeleteMessageBatch", MISSING, QueueName="q")
    s.cmd("DeleteMessageBatch", MISSING, Entries=eleven[:1])
    s.delete_batch("q", "r-1", INVALID_VALUE)
    s.delete_batch("q", [], EMPTY_BATCH)
    s.delete_batch("q", eleven, TOO_MANY_ENTRIES)
    s.counts("q", 0, 1, 0)
    s.delete_batch(
        "q",
        eleven[:10],
        batch([{"Id": "e0"}], [failed("e%d" % index, HANDLE_EXPIRED) for index in range(1, 10)]),
    )
    s.counts("q", 0, 0, 0)


# Feature: tags


@scenario(feature=True)
def tags(s):
    s.create("q")
    s.tags("q", {})
    s.tag("q", {"env": "prod", "team": "core"})
    s.tags("q", {"env": "prod", "team": "core"})
    s.tag("q", {"env": "dev", "cost": ""})
    s.tags("q", {"env": "dev", "team": "core", "cost": ""})
    s.untag("q", ["team", "nope"])
    s.tags("q", {"env": "dev", "cost": ""})
    s.untag("q", [])
    s.tag("q", {})
    s.untag("q", ["cost", "cost"])
    s.tags("q", {"env": "dev"})
    s.tag("q", {"Env": "upper", "note": 'spaces, "quotes" and \u00e9'})
    s.tags("q", {"env": "dev", "Env": "upper", "note": 'spaces, "quotes" and \u00e9'})
    s.counts("q", 0, 0, 0)


@scenario(feature=True)
def tags_validation(s):
    s.create("q")
    s.cmd("TagQueue", MISSING, QueueName="q")
    s.cmd("TagQueue", MISSING, QueueName="q", Tags=None)
    s.cmd("TagQueue", MISSING, Tags={"a": "b"})
    for value in [[], "x", 5, True, [{"a": "b"}]]:
        s.tag("q", value, INVALID_VALUE)
    for key in ["", "k" * 129, "a b", "k.1", "caf\u00e9", "k:1"]:
        s.tag("q", {key: "v"}, INVALID_VALUE)
    for value in [5, ["x"], True, {"v": 1}, "v" * 257, "\u00e9" * 129]:
        s.tag("q", {"key": value}, INVALID_VALUE)
    s.tag("q", {"good": "v", "bad key": "v"}, INVALID_VALUE)
    s.tag("q", {"good": "v", "other": 5}, INVALID_VALUE)
    s.tags("q", {})
    s.tag("q", {"k" * 128: "v" * 256, "accent": "\u00e9" * 128, "A-b_9": ""})
    s.tags("q", {"k" * 128: "v" * 256, "accent": "\u00e9" * 128, "A-b_9": ""})
    s.cmd("UntagQueue", MISSING, QueueName="q")
    s.cmd("UntagQueue", MISSING, TagKeys=["a"])
    for keys in ["accent", {"accent": 1}, 5, [5], [""], ["bad key"], ["accent", None], ["accent", "k" * 129]]:
        s.untag("q", keys, INVALID_VALUE)
    s.cmd("ListQueueTags", MISSING)
    s.cmd("ListQueueTags", INVALID_VALUE, QueueName=5)
    s.tags("q", {"k" * 128: "v" * 256, "accent": "\u00e9" * 128, "A-b_9": ""})
    s.untag("q", ["k" * 128, "accent"])
    s.tags("q", {"A-b_9": ""})


@scenario(feature=True)
def tags_limit(s):
    s.create("q")
    s.create("p")
    fifty = {"k%02d" % index: "v%d" % index for index in range(50)}
    s.tag("q", fifty)
    s.tags("q", fifty)
    s.tag("q", {"k50": "v"}, LIMIT)
    s.tag("q", {"k00": "new"})
    fifty["k00"] = "new"
    s.tag("q", {"k01": "changed", "k50": "v"}, LIMIT)
    s.tags("q", fifty)
    s.untag("q", ["k49"])
    del fifty["k49"]
    s.tag("q", {"k50": "v"})
    fifty["k50"] = "v"
    s.tags("q", fifty)
    s.tag("q", {"k51": "v"}, LIMIT)
    s.tag("p", {"t%02d" % index: "" for index in range(51)}, LIMIT)
    s.tags("p", {})
    s.tag("p", {"t%02d" % index: "" for index in range(49)})
    s.tag("p", {"t48": "x", "t49": "y", "t50": "z"}, LIMIT)
    s.tag("p", {"t48": "x", "t49": "y"})
    s.untag("p", ["t%02d" % index for index in range(48)])
    s.tags("p", {"t48": "x", "t49": "y"})


@scenario(feature=True)
def tags_follow_the_queue(s):
    s.tag("nope", {"a": "b"}, NO_QUEUE)
    s.untag("nope", ["a"], NO_QUEUE)
    s.cmd("ListQueueTags", NO_QUEUE, QueueName="nope")
    s.create("q", VisibilityTimeout=5)
    s.tag("q", {"a": "1"})
    s.create("q", VisibilityTimeout=5)
    s.create("q", ALREADY_EXISTS, VisibilityTimeout=6)
    s.send("q", "x", 1)
    s.purge("q")
    s.set("q", VisibilityTimeout=7)
    s.tags("q", {"a": "1"})
    s.counts("q", 0, 0, 0)
    s.create("other")
    s.tags("other", {})
    s.drop("q")
    s.cmd("ListQueueTags", NO_QUEUE, QueueName="q")
    s.create("q")
    s.tags("q", {})


@scenario(feature=True)
def tags_check_order(s):
    s.create("q")
    s.tag("q", {"k%02d" % index: "" for index in range(50)})
    s.cmd("TagQueue", MISSING, QueueName="nope")
    s.tag("nope", {"bad key": "v"}, INVALID_VALUE)
    s.tag("nope", [], INVALID_VALUE)
    s.tag("nope", {"k": "v"}, NO_QUEUE)
    s.tag("q", {"bad key": "v", "another": "v"}, INVALID_VALUE)
    s.tag("q", {"another": 5}, INVALID_VALUE)
    s.tag("q", {"another": "v"}, LIMIT)
    s.cmd("UntagQueue", MISSING, QueueName="nope")
    s.untag("nope", ["bad key"], INVALID_VALUE)
    s.untag("nope", "k", INVALID_VALUE)
    s.untag("nope", ["k"], NO_QUEUE)


# Feature: MaxMessageBytes


@scenario(feature=True)
def max_message_bytes_attribute(s):
    s.create("q")
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(MaxMessageBytes=1024), Counts=counts()), QueueName="q")
    s.create("p", MaxMessageBytes=10)
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(MaxMessageBytes=10), Counts=counts()), QueueName="p")
    for value in [0, 1025, -1, "10", True, 2.5]:
        s.create("r", INVALID_VALUE, MaxMessageBytes=value)
        s.set("q", INVALID_VALUE, MaxMessageBytes=value)
    s.create("low", MaxMessageBytes=1)
    s.create("high", MaxMessageBytes=1024)
    s.create("f.fifo", Fifo=True, MaxMessageBytes=7)
    s.counts("low")
    s.counts("high")
    s.counts("f.fifo")
    s.set("q", MaxMessageBytes=100)
    s.cmd("GetQueueAttributes", ok(Attributes=attrs(MaxMessageBytes=100), Counts=counts()), QueueName="q")
    s.set("q", MaxMessageBytes=1)
    s.set("q", MaxMessageBytes=1024, VisibilityTimeout=3)
    s.cmd(
        "GetQueueAttributes",
        ok(Attributes=attrs(MaxMessageBytes=1024, VisibilityTimeout=3), Counts=counts()),
        QueueName="q",
    )
    s.list(["f.fifo", "high", "low", "p", "q"])


@scenario(feature=True)
def max_message_bytes_enforced(s):
    s.create("q", MaxMessageBytes=10)
    s.send("q", "a" * 10, 1)
    s.send("q", "a" * 11, expect=INVALID_VALUE)
    s.send("q", "\u00e9" * 5, 2)
    s.send("q", "\u00e9" * 5 + "a", expect=INVALID_VALUE)
    s.send("q", "\u20ac" * 4, expect=INVALID_VALUE)
    s.set("q", MaxMessageBytes=5)
    s.send("q", "a" * 6, expect=INVALID_VALUE)
    s.send("q", "a" * 5, 3)
    s.counts("q", 3, 0, 0)
    s.receive("q", [msg(1, 1, "a" * 10), msg(2, 2, "\u00e9" * 5), msg(3, 3, "a" * 5)], MaxMessages=10)
    s.set("q", MaxMessageBytes=1024)
    s.send("q", "a" * 1024, 4)
    s.send("q", "a" * 1025, expect=INVALID_VALUE)
    s.create("one", MaxMessageBytes=1)
    s.send("one", "ab", expect=INVALID_VALUE)
    s.send("one", "\u00e9", expect=INVALID_VALUE)
    s.send("one", "", expect=INVALID_VALUE)
    s.send("one", "a", 5)


@scenario(feature=True)
def max_message_bytes_and_create_queue(s):
    s.create("q", MaxMessageBytes=10)
    s.create("q", MaxMessageBytes=10)
    s.cmd("CreateQueue", ALREADY_EXISTS, QueueName="q")
    s.create("q", ALREADY_EXISTS, MaxMessageBytes=11)
    s.create("q", ALREADY_EXISTS, MaxMessageBytes=1024)
    s.create("p")
    s.create("p", MaxMessageBytes=1024)
    s.set("p", MaxMessageBytes=5)
    s.cmd("CreateQueue", ALREADY_EXISTS, QueueName="p")
    s.create("p", MaxMessageBytes=5)
    s.counts("p")
    s.counts("q")


@scenario(feature=True)
def max_message_bytes_check_order(s):
    s.create("s", MaxMessageBytes=3)
    s.create("f.fifo", Fifo=True, MaxMessageBytes=3)
    s.send("nope", "toolong", expect=NO_QUEUE)
    s.send("s", "toolong", expect=INVALID_VALUE, GroupId="g")
    s.send("s", "abc", expect=UNSUPPORTED, GroupId="g")
    s.send("f.fifo", "toolong", expect=INVALID_VALUE)
    s.send("f.fifo", "toolong", expect=INVALID_VALUE, DelaySeconds=1)
    s.send("f.fifo", "abc", expect=UNSUPPORTED, DelaySeconds=1)
    s.send("f.fifo", "abc", expect=MISSING)
    s.fsend("f.fifo", "abc", "g", "d", 1, 1)
    s.fsend("f.fifo", "toolong", "g", "d", expect=INVALID_VALUE)
    s.fsend("f.fifo", "ab", "g", "d", 1, 1)
    s.counts("f.fifo", 1, 0, 0)
    s.counts("s", 0, 0, 0)


@scenario(feature=True)
def max_message_bytes_in_batches(s):
    s.create("q", MaxMessageBytes=4)
    s.send_batch(
        "q",
        [entry("a", Body="abcd"), entry("b", Body="abcde"), entry("c", Body="ab"), entry("d", Body="\u20ac\u20ac")],
        batch([sent("a", 1), sent("c", 2)], [failed("b", INVALID_VALUE), failed("d", INVALID_VALUE)]),
    )
    s.set("q", MaxMessageBytes=6)
    s.send_batch(
        "q",
        [entry("b", Body="abcde"), entry("d", Body="\u20ac\u20ac"), entry("e", Body="abcdefg")],
        batch([sent("b", 3), sent("d", 4)], [failed("e", INVALID_VALUE)]),
    )
    s.counts("q", 4, 0, 0)


@scenario(feature=True)
def max_message_bytes_and_dead_letter_moves(s):
    s.create("dlq", MaxMessageBytes=1)
    s.create("q", VisibilityTimeout=0, DeadLetterQueue="dlq", MaxReceiveCount=1)
    s.send("q", "hello", 1)
    s.receive("q", [msg(1, 1, "hello")])
    s.receive("q", [])
    s.counts("dlq", 1, 0, 0)
    s.send("dlq", "hello", expect=INVALID_VALUE)
    s.receive("dlq", [msg(1, 2, "hello")])
    s.delete("dlq", 2)
    s.counts("dlq", 0, 0, 0)


@scenario(feature=True)
def feature_stress_batches_and_tags(s):
    s.create("q", VisibilityTimeout=60, MaxMessageBytes=64)
    message = handle = 0
    tags = {}
    for round_number in range(150):
        size = 1 + round_number % 10
        entries = [entry("s%d" % index, Body="r%d-%d" % (round_number, index)) for index in range(size)]
        s.send_batch("q", entries, batch([sent("s%d" % index, message + index + 1) for index in range(size)]))
        s.receive(
            "q",
            [
                msg(message + index + 1, handle + index + 1, "r%d-%d" % (round_number, index),
                    sent=round_number, first=round_number)
                for index in range(size)
            ],
            MaxMessages=10,
        )
        deletes = [entry("d%d" % index, ReceiptHandle="r-%d" % (handle + index + 1)) for index in range(size)]
        deleted = [{"Id": "d%d" % index} for index in range(size)]
        if size == 10:
            s.delete_batch("q", deletes, batch(deleted))
        else:
            deletes.append(entry("old", ReceiptHandle="r-%d" % handle))
            s.delete_batch("q", deletes, batch(deleted, [failed("old", HANDLE_EXPIRED if handle else HANDLE_INVALID)]))
        message += size
        handle += size
        key = "k%d" % (round_number % 60)
        if key in tags and round_number % 3 == 0:
            del tags[key]
            s.untag("q", [key])
        elif key in tags or len(tags) < 50:
            tags[key] = "round-%d" % round_number
            s.tag("q", {key: tags[key]})
        else:
            s.tag("q", {key: "v"}, LIMIT)
        if round_number % 10 == 9:
            s.tags("q", dict(tags))
            s.counts("q", 0, 0, 0)
        s.tick(1, round_number + 1)


# Runner


def encode(script):
    data = b"\n".join(line for _, line, _ in script.steps)
    if script.steps and script.final_newline:
        data += b"\n"
    return data


def run_batch(binary, script):
    try:
        done = subprocess.run(
            [binary],
            input=encode(script),
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            timeout=TIMEOUT,
        )
    except subprocess.TimeoutExpired:
        return None, "timed out after %d s" % TIMEOUT
    except OSError as error:
        return None, "cannot run the binary: %s" % error
    output = done.stdout
    if output and not output.endswith(b"\n"):
        return None, "the last response line does not end with a newline"
    lines = output.split(b"\n")[:-1] if output else []
    return lines, None if done.returncode == 0 else "exit status %d" % done.returncode


def run_interactive(binary, script):
    try:
        process = subprocess.Popen(
            [binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL
        )
    except OSError as error:
        return None, "cannot run the binary: %s" % error
    responses = queue.Queue()

    def pump():
        for line in process.stdout:
            responses.put(line)
        responses.put(None)

    threading.Thread(target=pump, daemon=True).start()
    deadline = time.monotonic() + TIMEOUT
    lines, problem = [], None
    try:
        for _, line, _ in script.steps:
            process.stdin.write(line + b"\n")
            process.stdin.flush()
            response = responses.get(timeout=max(deadline - time.monotonic(), 0.01))
            if response is None:
                problem = "the process closed stdout before answering"
                break
            lines.append(response.rstrip(b"\n"))
        process.stdin.close()
        if problem is None:
            extra = responses.get(timeout=max(deadline - time.monotonic(), 0.01))
            if extra is not None:
                lines.append(extra.rstrip(b"\n"))
            status = process.wait(timeout=max(deadline - time.monotonic(), 0.01))
            if status != 0:
                problem = "exit status %d" % status
    except queue.Empty:
        problem = "no response within %d s: responses are not written line by line" % TIMEOUT
    except subprocess.TimeoutExpired:
        problem = "the process did not exit at end of input"
    except OSError as error:
        problem = "cannot write to the process: %s" % error
    finally:
        process.kill()
        process.wait()
    return lines, problem


def check(script, lines, problem):
    if lines is None:
        return problem
    for index, (label, _, expected) in enumerate(script.steps):
        where = "step %d (%s)" % (index + 1, label)
        if index >= len(lines):
            return "%s: no response%s" % (where, " (%s)" % problem if problem else "")
        try:
            actual = json.loads(lines[index].decode("utf-8"))
        except ValueError:
            return "%s: the response is not a JSON line: %r" % (where, lines[index][:80])
        found = mismatch(expected, actual)
        if found:
            return "%s: %s" % (where, found)
    if len(lines) > len(script.steps):
        return "expected %d response lines, got %d" % (len(script.steps), len(lines))
    return problem


def run_scenario(binary, function, interactive):
    script = Script()
    function(script)
    lines, problem = (run_interactive if interactive else run_batch)(binary, script)
    return check(script, lines, problem)


def main(argv):
    global FEATURE
    arguments = list(argv[1:])
    as_json = "--json" in arguments
    FEATURE = "--feature" in arguments
    pattern = None
    if "-k" in arguments:
        position = arguments.index("-k")
        pattern = arguments[position + 1] if position + 1 < len(arguments) else ""
        del arguments[position : position + 2]
    paths = [argument for argument in arguments if not argument.startswith("--")]
    if len(paths) != 1:
        sys.stderr.write(__doc__)
        return 2
    selected = [
        (name, function, interactive)
        for name, function, feature, interactive in SCENARIOS
        if (FEATURE or not feature) and (pattern is None or pattern in name)
    ]
    failures = []
    for name, function, interactive in selected:
        problem = run_scenario(paths[0], function, interactive)
        if problem:
            failures.append(name)
        if not as_json:
            print("FAIL %s: %s" % (name, problem) if problem else "PASS %s" % name, flush=True)
    passed = len(selected) - len(failures)
    if as_json:
        print(json.dumps({"passed": passed, "total": len(selected), "failed": failures}))
    else:
        print("passed %d/%d" % (passed, len(selected)))
    return 0 if not failures and selected else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
