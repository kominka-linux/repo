#!/usr/bin/env python3
"""List every unique `seed CMD args…` invocation across .ysh files.

Prints one line per unique call (without the leading "seed " prefix), sorted.

Uses `ysh -n --ast-format text` for AST-based detection — no regex parsing of
ysh source text.

Usage:
  python3 scripts/seed-calls.py [file.ysh ...]
  With no args, globs **/*.ysh from cwd.
"""

import re
import sys
import subprocess
from glob import glob

_RE_COMPACT_SRCLINE = re.compile(
    r'line:\(SourceLine line_num:(\d+) content:"((?:[^"\\]|\\.)*)"'
)
_RE_LINE_NUM = re.compile(r'^line_num:(\d+)$')
_RE_CONTENT  = re.compile(r'^content:"((?:[^"\\]|\\.)*)"$')
_RE_BLAME_COMPACT = re.compile(
    r'^blame_tok:\(Token id:(\S+) length:(\d+) col:(\d+) line:(.*)\)$'
)


def _unescape(s):
    return (s.replace('\\"', '"')
             .replace('\\\\', '\\')
             .replace('\\n', '\n')
             .replace('\\t', '\t'))


def _seed_calls(ast_text):
    """Yield (line_num, col, raw_content) for every `seed` command.Simple."""
    last_line_num = 0
    last_content  = ''
    after_simple  = False
    in_blame      = False
    got_static_id = False
    blame_length  = 0
    blame_col     = 0
    in_srcline    = False

    def _emit(ln, col, content):
        if content[col:col + 4] == 'seed' and (
            len(content) <= col + 4 or content[col + 4] in (' ', '\t', '\n', '\r')
        ):
            yield ln, col, content

    for raw in ast_text.split('\n'):
        s = raw.strip()

        if not in_blame:
            m = _RE_COMPACT_SRCLINE.search(s)
            if m:
                last_line_num = int(m.group(1))
                last_content  = _unescape(m.group(2))
            elif not in_srcline:
                m = _RE_LINE_NUM.match(s)
                if m:
                    last_line_num = int(m.group(1))
                m = _RE_CONTENT.match(s)
                if m:
                    last_content = _unescape(m.group(1))

        if s.endswith('(command.Simple'):
            after_simple = True
            in_blame     = False
            in_srcline   = False
            continue

        if after_simple:
            after_simple = False
            if s == 'blame_tok:(Token':
                in_blame      = True
                got_static_id = False
                blame_length  = 0
                blame_col     = 0
                in_srcline    = False
            else:
                m = _RE_BLAME_COMPACT.match(s)
                if m:
                    tok_id, length, col, line_part = (
                        m.group(1), int(m.group(2)), int(m.group(3)), m.group(4)
                    )
                    if tok_id == 'Lit_Chars':
                        ms = _RE_COMPACT_SRCLINE.search(line_part)
                        if ms:
                            ln      = int(ms.group(1))
                            content = _unescape(ms.group(2))
                            last_line_num = ln
                            last_content  = content
                            yield from _emit(ln, col, content)
                        elif line_part.startswith('...') and last_content:
                            yield from _emit(last_line_num, col, last_content)
            continue

        if in_blame and not in_srcline:
            if s == 'id:Lit_Chars':
                got_static_id = True
            elif s.startswith('id:'):
                in_blame = False
            elif s.startswith('length:'):
                blame_length = int(s[7:])
            elif s.startswith('col:'):
                blame_col = int(s[4:])
            elif s == 'line:(SourceLine':
                in_srcline = True
            elif s.startswith('line:(SourceLine '):
                m = _RE_COMPACT_SRCLINE.search(s)
                if m:
                    ln      = int(m.group(1))
                    content = _unescape(m.group(2))
                    last_line_num = ln
                    last_content  = content
                    if got_static_id:
                        yield from _emit(ln, blame_col, content)
                in_blame = False
            elif s.startswith('line:...'):
                if got_static_id and last_content:
                    yield from _emit(last_line_num, blame_col, last_content)
                in_blame = False
            continue

        if in_srcline:
            m = _RE_LINE_NUM.match(s)
            if m:
                last_line_num = int(m.group(1))
            m = _RE_CONTENT.match(s)
            if m:
                last_content = _unescape(m.group(1))
                if in_blame and got_static_id:
                    yield from _emit(last_line_num, blame_col, last_content)
                in_blame   = False
                in_srcline = False


def _trim_command(text):
    """Return the leading command from `text`, stopping at the first
    unbalanced ), }, ;, |, <, > or bare { not inside a quoted string.

    ${ opens a variable expansion (tracked by depth); a bare { that is
    not immediately preceded by $ is a ysh block opener → stop.

    When stopping at > the trailing fd digit (e.g. "2" from "2>/dev/null")
    is stripped.  Trailing backslash line-continuations are also stripped.
    """
    depth_paren   = 0
    depth_brace   = 0
    in_single     = False
    in_double     = False
    stopped_at_gt = False
    i = 0
    while i < len(text):
        c = text[i]
        if in_single:
            if c == "'":
                in_single = False
        elif in_double:
            if c == '\\':
                i += 1  # skip escaped char
            elif c == '"':
                in_double = False
        else:
            if c == "'":
                in_single = True
            elif c == '"':
                in_double = True
            elif c == '(':
                depth_paren += 1
            elif c == ')':
                if depth_paren > 0:
                    depth_paren -= 1
                else:
                    break
            elif c == '{':
                # ${ = variable expansion (balanced); bare { = block opener
                if i > 0 and text[i - 1] == '$':
                    depth_brace += 1
                else:
                    break
            elif c == '}':
                if depth_brace > 0:
                    depth_brace -= 1
                else:
                    break
            elif c in (';', '|', '<'):
                break
            elif c == '>':
                stopped_at_gt = True
                break
            elif c == '&' and i + 1 < len(text) and text[i + 1] == '&':
                break
        i += 1
    result = text[:i].rstrip()
    # Strip fd number left by stopping at > (e.g. "2" from "cmd 2>/dev/null").
    # Only strip when the trailing digit(s) form the entire last word
    # (preceded by a space), so we don't mangle args like sort's -k1,1.
    if stopped_at_gt and result and result[-1].isdigit():
        last_word_start = result.rfind(' ') + 1
        if result[last_word_start:].isdigit():
            result = result[:last_word_start].rstrip()
    # Strip line-continuation backslash
    if result.endswith('\\'):
        result = result[:-1].rstrip()
    return result


def collect(path, results):
    r = subprocess.run(
        ['ysh', '-n', '--ast-format', 'text', path],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        print(f'{path}: ysh parse error', file=sys.stderr)
        return
    for line_num, col, content in _seed_calls(r.stdout):
        snippet = _trim_command(content[col:].rstrip('\n'))
        results.add(snippet[5:])  # strip leading "seed "


def main():
    paths = sys.argv[1:] or sorted(glob('**/*.ysh', recursive=True))
    if not paths:
        print('seed-calls: no .ysh files found', file=sys.stderr)
        return
    results = set()
    for path in paths:
        collect(path, results)
    for line in sorted(results):
        print(line)


if __name__ == '__main__':
    main()
