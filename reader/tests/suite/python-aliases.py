"""Write python-aliases.jsonl: what CPython leaves in a JSON file after a program
stores through a second name for part of it, for aliases.rs.

    python3 reader/tests/suite/python-aliases.py > reader/tests/suite/python-aliases.jsonl

`must` rows are forms the evaluator follows; the rest are where one object under
two names is easy to get wrong, and may be refused but never mispredicted.
"""

import json
import os
import subprocess
import sys
import tempfile

START = {
    "spec": {"containers": [{"env": [{"name": "A"}], "args": ["x"]}]},
    "parts": [
        {"id": "a", "size": [1, 2]},
        {"id": "b", "size": [3, 4]},
        {"id": "a", "size": [1, 2]},
    ],
    "n": 1,
    "m": {"k": [7, 8]},
}
HEAD = "import json\nj=json.load(open('j.json'))\n"
TAIL = "\njson.dump(j,open('j.json','w'))\n"

MUST = [
    "c=j['spec']['containers'][0]\nc['env']=[]",
    "c=j['spec']\nc['containers'][0]['args']=['y']",
    "for q in j['parts']:\n    q['size'][0]=9",
    "for q in j['parts']:\n    if q['id']=='b':\n        q['size'][0]=9",
    "for q in j['parts']:\n    q['w']=1",
    "p=j['parts']\nq=p[1]\nq['id']='z'",
    "c=j['m']\nd=c['k']\nd[0]=7\nc['z']=1",
    "q=j['parts'][0]\nq['size'][1]=5",
    "a=j['parts'][0]\nb=j['parts'][2]\na['x']=1\nb['y']=2",
    "c=j['m']\nc['k']=[9]\nc['k'][0]=8",
    "q=j['parts'][-1]\nq['last']=True",
    "q=j['parts'][0]\nr=j['parts'][0]\nq['a1']=1\nr['a2']=2",
    "for part in j['parts']:\n    for n,_ in enumerate(part['size']):\n        part['size'][n]=part['size'][n]*2",
    "c=j['m']\nj['m']={'new':1}\nc['old']=2",
    "c=j['m']['k']\nj['m']['k']=[5]\nc[0]=6",
]
MAY = [
    "p=j['parts']\nq=p[0]\np.insert(0,{'id':'n','size':[0,0]})\nq['id']='moved'",
    "q=j['parts'][0]\nj['parts'].pop(0)\nq['id']='gone'",
    "q=j['parts'][0]\nj['parts'].sort(key=lambda x:x['size'][0], reverse=True)\nq['id']='s'",
    "c=j['m']\nj=dict(j)\nc['t']=1",
    "c=j['m']\nk={'x':c}\nk['x']['z']=1",
    "c=j['m']\nc=dict(c)\nc['u']=1",
    "for q in j['parts']:\n    j['parts'][0]['size'][0]=q['size'][0]+1",
    "for q in j['parts']:\n    q['size']=[0]\n    q['size'].append(1)",
    "for q in j['parts']:\n    if q['id']=='a':\n        j['n']+=1\n        q['c']=j['n']",
    "for i,q in enumerate(j['parts']):\n    q['i']=i",
    "for f in [j['m']]:\n    f['k'][1]=0",
    "for q in j['parts']:\n    for s in [q['size']]:\n        s[0]=0",
    "def f(d):\n    d['fx']=1\nf(j['m'])",
    "c=j['m']\ndef g():\n    c['gx']=1\ng()",
    "c=j['m']\nc['k'].append(3)\nc['q']=1",
    "c=j['m']\ndel c['k']\nc['z']=1",
    "x=j['parts']\ny=x\ny[0]={'id':'new'}\nx[1]['id']='via-x'",
    "c=j['spec']['containers']\nc[0]=c[0]\nc[0]['same']=1",
    "a=j['m']\na['self']=a['k']\na['self'][0]=99",
    "j['m']['self']=j['m']['k']\nj['m']['self'][0]=99",
    "q=j['parts'][0]\nj['parts'][1]=q\nq['both']=1",
    "q=j['parts'][0]\nj['spare']=q\nj['spare']['id']='shared'",
    "for q in j['parts']:\n    j['parts'].append(1) if False else None\n    q['ok']=1",
    "for q in j['parts']:\n    j['last']=q\nj['last']['id']='end'",
    "c=j['m']\nfor q in j['parts']:\n    c=q\nc['after']=1",
    "q=j['parts'][0]\nq=j['parts'][1]\nq['second']=1",
    "c=j['m']\nif j['n']==1:\n    c=j['spec']\nc['picked']=1",
    "import sys\nc=j['m']\nif len(sys.argv)>5:\n    c=j['spec']\nc['picked']=1",
    "import sys\nc=j['m']\nif len(sys.argv)>5:\n    c['k']=[0]\nelse:\n    c['k']=[1]\nc['done']=1",
    "p=j['parts']\np[0],p[1]=p[1],p[0]\nq=p[0]\nq['swapped']=1",
    "q=j['parts'][0]\nj['parts']=[q]\nq['kept']=1",
    "q=j['parts'][0]['size']\nq+=[3]\nq[0]=0",
    "import sys\nc=j['m']\nif len(sys.argv)>5:\n    c=j['spec']\nc.update(z=1)",
    "import sys\nc=j['m']\nif len(sys.argv)>5:\n    c=j['spec']\nc.setdefault('s',1)",
    "import sys\nc=j['m']\nif len(sys.argv)>5:\n    c=j['spec']\nc['k'].append(5)",
    "import sys\nc=j['m']\nif len(sys.argv)>5:\n    c=j['spec']\ndel c['k']",
    "import sys\nc=j['m']\nif len(sys.argv)>5:\n    c=j['spec']\nc['k'][0]=5",
    "import sys\nc=j['parts']\nif len(sys.argv)>5:\n    c=j['m']['k']\nc[0]=5",
    "import sys\nc=j['parts']\nif len(sys.argv)>5:\n    c=j['m']['k']\nc+=[1]",
    "import sys\nc=j['parts']\nif len(sys.argv)>5:\n    c=j['m']['k']\nc.clear()",
    "c=j['m']\nj['m2']=c\nj['m2']['k'].append(7)",
    "c=j['m']['k']\nj['parts'][0]['size']=c\nc.append(7)",
    "x=[j['m']]\nj['box']=x\nx[0]['in']=1",
    "j['n2']=j['m']['k']\nj['m']['k'].append(8)",
    "j['copy']=dict(j['m'])\nj['copy']['k'][0]=4",
    "j['copy']=list(j['parts'])\nj['copy'][0]['id']='c'",
    "j['env2']=[e for e in j['spec']['containers'][0]['env']]\nj['env2'][0]['name']='B'",
    "c=j['spec']['containers'][0]\nc['env']=[e for e in c['env'] if e['name']!='Z']+[{'name':'N'}]",
    "c=j['spec']['containers'][0]\nold=c['env']\nc['env']=[e for e in old]\nold[0]['name']='via-old'",
    "d=j['m']\nd['k']=[x*2 for x in d['k']]",
    "x=[1]\nj['a']=x\nj['b']=x\nj['a'][0]=5",
    "x=[1]\nj['d']={'a':x,'b':x}\nj['d']['a'][0]=5",
    "g=[[0]*2]*2\ng[0][0]=1\nj['g']=g",
    "x=[1]\nd={'a':x,'b':x}\nd['a'][0]=5\nj['d']=d",
    "x=[1]\ny=[x,x]\ny[0][0]=5\nj['y']=y",
    "x=[1]\ny=[x,x]\nx[0]=5\nj['y']=y",
    "x=[1]\ny=(x,x)\ny[0].append(2)\nj['y']=list(y)",
    "j['g']=[[0]*2]*2\nj['g'][1][1]=7",
    "row=[0,0]\nj['g']=[row for _ in range(2)]\nj['g'][0][0]=1",
    "j['g']=[[0,0] for _ in range(2)]\nj['g'][0][0]=1",
    "d={}\nj['parts'][0]['meta']=d\nj['parts'][1]['meta']=d\nj['parts'][0]['meta']['k']=1",
    "j['parts'][1]['size']=j['parts'][0]['size']\nj['parts'][0]['size'][0]=0",
    "j['parts'][1]['size']=list(j['parts'][0]['size'])\nj['parts'][0]['size'][0]=0",
    "x=[1]\ny=[]\ny.append(x)\ny.append(x)\ny[0][0]=5\nj['y']=y",
    "x=[1]\nd={}\nd.update(a=x,b=x)\nd['a'][0]=5\nj['d']=d",
    "x=[1]\nd={}\nd.setdefault('a',x)\nd.setdefault('b',x)\nd['a'][0]=5\nj['d']=d",
    "x=[1]\ny=[0,0]\ny[0]=x\ny[1]=x\ny[0][0]=5\nj['y']=y",
    "x=[1]\ny=[x]+[x]\ny[0][0]=5\nj['y']=y",
    "x=[1]\ny=[x]*2\ny[0][0]=5\nj['y']=y",
    "x=[1]\ny=[x for _ in range(2)]\ny[0][0]=5\nj['y']=y",
    "x=[1]\ny=dict(a=x,b=x)\ny['a'][0]=5\nj['y']=y",
    "x=[1]\ny={k:x for k in 'ab'}\ny['a'][0]=5\nj['y']=y",
    "x=[1]\ny=list((x,x))\ny[0][0]=5\nj['y']=y",
    "x=[1]\ny=[x,x]\nz=y[:]\nz[0][0]=5\nj['z']=z",
    "x=[1]\ny=[x,[1]]\ny[0][0]=5\nj['y']=y",
]


def ran(body):
    with tempfile.TemporaryDirectory() as tmp:
        with open(os.path.join(tmp, "j.json"), "w") as f:
            json.dump(START, f)
        done = subprocess.run([sys.executable, "-c", HEAD + body + TAIL], cwd=tmp, capture_output=True)
        if done.returncode != 0:
            return None
        return open(os.path.join(tmp, "j.json")).read()


for must, bodies in ((True, MUST), (False, MAY)):
    for body in bodies:
        print(json.dumps({"must": must, "start": json.dumps(START), "program": HEAD + body + TAIL, "left": ran(body)}))
