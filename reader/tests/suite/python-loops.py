"""Write python-loops.jsonl: what CPython leaves in `o.txt` after a `while`, an
ordering or identity comparison, or a list's `pop`, `index` and `count`, for
loops.rs.

    python3 reader/tests/suite/python-loops.py > reader/tests/suite/python-loops.jsonl

`must` rows are forms the evaluator follows; the rest may be refused but never
mispredicted. A program that raises leaves nothing, and its row says so.
"""

import json
import os
import subprocess
import sys
import tempfile

TEXT = "//! head\n//! more\nfn a() {}\n    body\n    more\n/// doc\n/// doc2\nfn b() {}\n\n"
HEAD = "s=open('t.txt').read()\nL=s.split('\\n')\n"


def out(expr):
    return f"\nopen('o.txt','w').write(str({expr}))\n"


def outs(items):
    """Each of a list's values as text, joined: a list's own repr is another matter."""
    return f"\nopen('o.txt','w').write(','.join([str(v) for v in {items}]))\n"


MUST = [
    "n=0\nwhile n<3:\n    n+=1" + out("n"),
    "j=3\nwhile j<len(L) and L[j].startswith(' '): j+=1" + out("j"),
    "i=0\nwhile L[i].startswith('//!'): i+=1" + out("i"),
    "while '//' in s: s=s.replace('//','/')" + out("s"),
    "while L and L[-1]=='':\n    L.pop()" + out("'|'.join(L)"),
    "start=s.index('fn b')\nwhile True:\n    prev=s.rfind('\\n',0,start-1)\n    line=s[prev+1:start]\n    if line.startswith('///'): start=prev+1\n    else: break" + out("start"),
    "ls=['a',None,None,'b']\nj=2\nwhile ls[j] is None: j -= 1" + out("j"),
    "i=0\nout_=[]\nwhile i<len(L):\n    if L[i].startswith('///'):\n        i+=1\n        continue\n    out_.append(L[i])\n    i+=1" + out("'|'.join(out_)"),
    "i=0\nwhile i<10:\n    i+=1\n    if i==4: break\nelse:\n    i=99" + out("i"),
    "i=0\nwhile i<3:\n    i+=1\nelse:\n    i=99" + out("i"),
    "i=0\nt=0\nwhile i<3:\n    j=0\n    while j<i:\n        t+=1\n        j+=1\n    i+=1" + out("t"),
    outs("[1<2, 2<1, 1<=1, 2>=3, 'a'<'b', 'B'<'a', 'ab'<'abc', [1,2]<[1,3], [1]<[1,0]]"),
    outs("[1<1.5, 2>1.5, 1<=1.0, 1.0>=1, 3<2.9999, 0.1+0.2>0.3, -1<0.0]"),
    outs("[9007199254740993<9007199254740992.0, 9007199254740993>9007199254740992.0, 9007199254740993<9007199254740994.0, 4611686018427387904<1e30, -4611686018427387904>-1e30, 4611686018427387904>4611686018427387903.5]"),
    outs("[None is None, None is not None, 'a' is None, 0 is not None, [] is None, True is True, True is False, False is not None]"),
    outs("[1<2<3, 1<3<2, 3>2>1, 1<=1<2]"),
    out("L.index('fn a() {}')"),
    out("L.index('    more', 2)"),
    out("L.count('')"),
    out("['a','b','a'].index('a', 1)"),
    "x=L.pop()" + outs("[x, len(L)]"),
    "x=L.pop(0)" + outs("[x, L[0]]"),
    "x=L.pop(-2)" + outs("[x, len(L)]"),
    "f=open('o.txt','w')\nn=f.write('héllo')\nf.write(str(n))\nf.close()",
]
MAY = [
    outs("[float('nan')<1, 1<float('nan'), float('nan')>=float('nan'), float('inf')>10**18, -float('inf')<-10**18]"),
    outs("[1e999>4611686018427387904, -1e999<-4611686018427387904, 1e999-1e999<1, 1<1e999-1e999, 2**62<1e30]"),
    "i=0\nwhile i<3:\n    i+=1\n    continue\n    i=50" + out("i"),
    "i=0\nwhile True:\n    i+=1\n    if i>5: break" + out("i"),
    "import sys\ni=0\nwhile i<len(sys.argv)*3:\n    i+=1" + out("i"),
    "import os\ni=0\nwhile i<3 and os.environ.get('NOPE') is None:\n    i+=1" + out("i"),
    "import os\ni=0\nwhile os.environ.get('NOPE'):\n    i+=1" + out("i"),
    "i=0\nwhile i<3:\n    i+=1\n    if i==2:\n        import os\n        if os.environ.get('NOPE'): break" + out("i"),
    "i=0\nwhile i<20000: i+=1" + out("i"),
    "def f():\n    i=0\n    while i<5:\n        i+=1\n        if i==3: return i\n    return -1" + out("f()"),
    "i=5\nwhile i:\n    i-=1" + out("i"),
    "xs=[1,2,3]\nwhile xs: xs.pop(0)" + out("len(xs)"),
    "xs=[3,1,2]\ni=0\nwhile i<len(xs)-1:\n    if xs[i]>xs[i+1]:\n        xs[i],xs[i+1]=xs[i+1],xs[i]\n        i=0\n    else: i+=1" + outs("xs"),
    "d={'a':1}\nwhile len(d)<4: d[str(len(d))]=len(d)" + out("sorted(d)"),
    "i=0\ntry:\n    while True:\n        L[i]\n        i+=1\nexcept IndexError:\n    pass" + out("i"),
    "i=0\nwhile (i:=i+1)<4: pass" + out("i"),
    "it=iter(L)\nn=0\nwhile next(it, None) is not None: n+=1" + out("n"),
    outs("[True<2, False<True, 1<True, 2>True]"),
    outs("['a'<1]"),
    outs("[None<1]"),
    outs("[(1,'a')<(1,'b'), (1,2)<(1,2,3), ()<(0,)]"),
    outs("[1.5<'a']"),
    outs("[1 is 1.0, 'a' is 'a', [] is [], 1 is not None, None is 0]"),
    out("x=1") ,
    out("[1,True,1.0].index(True)"),
    out("[1,2,1.0].count(1)"),
    out("['a','b'].index('z')"),
    out("L.index('fn a() {}', -3)"),
    out("L.index('//! head', -100)"),
    out("['a',1].index('a')"),
    out("[].pop()"),
    out("[1].pop(5)"),
    "x=[1,2,3]\ny=x\ny.pop()" + outs("x"),
    "x=[[1],[2]]\ny=x.pop()\ny.append(9)" + outs("[x,y]"),
    "x=[[1],[2]]\ny=x[1]\nz=x.pop()\nz.append(9)" + outs("[x,y,z]"),
    "x={'k':[1,2]}\ny=x['k']\nx['k'].pop()" + outs("[x,y]"),
    "f=open('o.txt','w')\nwhile f.write('x')<2 and False: pass\nf.close()",
    "n=0\nwhile open('o.txt','a').write('x') and n<2: n+=1",
    out("max(3, 2.5) > 2 and min('a','b') < 'b'"),
    out("sorted([2,1.5,3]) < [1.5,2,4]"),
    outs("[2 < 3.0 < 4, 2.0 <= 2 <= 2.0]"),
]


def ran(body):
    with tempfile.TemporaryDirectory() as tmp:
        with open(os.path.join(tmp, "t.txt"), "w") as f:
            f.write(TEXT)
        done = subprocess.run([sys.executable, "-c", HEAD + body], cwd=tmp, capture_output=True, timeout=20)
        path = os.path.join(tmp, "o.txt")
        if done.returncode != 0 or not os.path.exists(path):
            return None
        return open(path).read()


for must, bodies in ((True, MUST), (False, MAY)):
    for body in bodies:
        print(json.dumps({"must": must, "text": TEXT, "program": HEAD + body, "left": ran(body)}))
