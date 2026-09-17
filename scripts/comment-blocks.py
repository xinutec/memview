#!/usr/bin/env python3
"""Number the comment blocks of a source file; apply decisions to them.

  blocks.py list FILE            -> "#id L<start>-<end> (<n> lines)" + the block text
  blocks.py apply FILE JSON      -> JSON maps id -> "cut" | "keep" | ["line", ...]
                                    (replacement lines, written with the block's indent
                                    and comment marker)

A block is a run of whole-line comments (Rust: //, ///, //! ; TS/SCSS: //, /** .. */,
/* .. */ ; HTML: <!-- .. -->). Trailing comments on code lines are not blocks.
"""
import json, re, sys

def kind_of(path):
    if path.endswith(('.rs',)): return 'rs'
    if path.endswith(('.ts', '.scss', '.mjs', '.js')): return 'ts'
    if path.endswith('.html'): return 'html'
    if path.endswith('.dhall'): return 'dhall'
    if path.endswith('.sh'): return 'sh'
    raise SystemExit(f'unknown kind: {path}')

def blocks(lines, kind):
    """Yield (start, end, marker, indent) for each comment block, end exclusive."""
    i, n = 0, len(lines)
    while i < n:
        s = lines[i].lstrip()
        indent = lines[i][: len(lines[i]) - len(s)]
        if kind in ('rs', 'ts', 'sh') and (s.startswith('//') or (kind == 'sh' and s.startswith('#') and not s.startswith('#!'))):
            marker = '#' if kind == 'sh' else re.match(r'//[/!]?', s).group(0)
            j = i
            while j < n:
                t = lines[j].lstrip()
                if not t.startswith(marker) or (kind != 'sh' and re.match(r'//[/!]?', t).group(0) != marker): break
                if kind == 'sh' and t.startswith('#!'): break
                if lines[j][: len(lines[j]) - len(t)] != indent: break
                j += 1
            yield i, j, marker, indent
            i = j
            continue
        if kind in ('ts',) and (s.startswith('/**') or s.startswith('/*')):
            j = i
            while j < n and '*/' not in lines[j]:
                j += 1
            j = min(j + 1, n)
            yield i, j, '/**' if s.startswith('/**') else '/*', indent
            i = j
            continue
        if kind == 'html' and s.startswith('<!--'):
            j = i
            while j < n and '-->' not in lines[j]:
                j += 1
            j = min(j + 1, n)
            yield i, j, '<!--', indent
            i = j
            continue
        if kind == 'dhall' and s.startswith('{-'):
            j = i
            while j < n and '-}' not in lines[j]:
                j += 1
            j = min(j + 1, n)
            yield i, j, '{-', indent
            i = j
            continue
        i += 1

def render(marker, indent, text, kind):
    if marker in ('//', '///', '//!', '#'):
        return [f'{indent}{marker} {t}'.rstrip() if t else f'{indent}{marker}' for t in text]
    if marker == '/**':
        if len(text) == 1:
            return [f'{indent}/** {text[0]} */']
        return [f'{indent}/**'] + [f'{indent} * {t}'.rstrip() for t in text] + [f'{indent} */']
    if marker == '/*':
        return [f'{indent}/*'] + [f'{indent} * {t}'.rstrip() for t in text] + [f'{indent} */']
    if marker == '<!--':
        if len(text) == 1:
            return [f'{indent}<!-- {text[0]} -->']
        return [f'{indent}<!-- {text[0]}'] + [f'{indent}     {t}'.rstrip() for t in text[1:-1]] + [f'{indent}     {text[-1]} -->']
    if marker == '{-':
        return [f'{indent}{{-  {text[0]}'] + [f'{indent}    {t}'.rstrip() for t in text[1:]] + [f'{indent}-}}']
    raise SystemExit(marker)

def main():
    cmd, path = sys.argv[1], sys.argv[2]
    kind = kind_of(path)
    lines = open(path).read().split('\n')
    found = list(blocks(lines, kind))
    if cmd == 'list':
        for k, (a, b, marker, indent) in enumerate(found):
            print(f'#{k} L{a+1}-{b} ({b-a} lines)')
            for l in lines[a:b]:
                print('    ' + l)
        return
    if cmd == 'apply':
        decisions = json.load(open(sys.argv[3]))
        out = []
        pos = 0
        for k, (a, b, marker, indent) in enumerate(found):
            out.extend(lines[pos:a])
            d = decisions.get(str(k), 'keep')
            if d == 'keep':
                out.extend(lines[a:b])
            elif d == 'cut':
                # Drop a blank line left directly above a now-orphaned gap between two blanks.
                pass
            else:
                out.extend(render(marker, indent, d, kind))
            pos = b
        out.extend(lines[pos:])
        # Collapse runs of 3+ blank lines left by cuts to 2 (rustfmt/prettier fix the rest).
        text = re.sub(r'\n{3,}', '\n\n', '\n'.join(out))
        open(path, 'w').write(text)
        missing = [k for k in decisions if int(k) >= len(found)]
        if missing: print('unknown ids:', missing, file=sys.stderr)
        return
    raise SystemExit('list|apply')

main()
