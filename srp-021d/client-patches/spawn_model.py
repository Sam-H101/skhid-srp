"""A model of match_time_vi_spawn_rule::tick, the original and the patched one
(docs/notes/battery_spawn.md), with and without rollbacks.

    python spawn_model.py
"""

import random
M=1<<32
def lcg(x): return (x*0x8088405+1)%M
class Rule:
    def __init__(s,seed,start=50,spawn=30000,var=10000,count=5):
        s.rnd=seed; s.start=start; s.spawn=spawn; s.var=var; s.count=count; s.avail=0; s.next=start+spawn; s.log=[]
    def state(s): return (s.rnd,s.avail)               # what serialize keeps
    def load(s,st): s.rnd,s.avail=st
def tick_orig(s,t):
    if s.avail>=s.count: return
    if ((s.var+t-s.start)%M)%s.spawn==0:
        s.rnd=lcg(s.rnd); s.next=(t+((s.rnd*2*s.var)>>32))%M
    if t>=s.next:
        s.log.append(t); s.avail+=1; s.next+=s.spawn
def tick_new(s,t):
    if s.avail>=s.count: return
    k,rem=divmod((s.var-s.start+t)%M,s.spawn)
    if rem==0: s.rnd=lcg(s.rnd)
    if k<=s.avail: return
    due=(t-rem+((s.rnd*2*s.var)>>32))%M
    if t>=due: s.log.append(t); s.avail+=1
def run(tick,seed,rollback):
    r=Rule(seed); hist={}; T=50; rng=random.Random(seed)
    while T<=600000:
        hist[T]=r.state(); tick(r,T); T+=50
        if rollback and rng.random()<0.8:          # a late input: back up to 50..500 ms
            back=T-50*rng.randint(1,10)
            if back in hist:
                r.load(hist[back]); r.log=[x for x in r.log if x<back]
                for t in range(back,T,50): hist[t]=r.state(); tick(r,t)
    return r.log
for seed in range(2000):
    a=run(tick_orig,seed,False); b=run(tick_new,seed,False); c=run(tick_new,seed,True)
    assert a==b==c, (seed,a,b,c)
    assert len(a)==5
print('same times without rollbacks, kept under rollbacks; e.g.', run(tick_new,7,True))
print('original under rollbacks:', [len(run(tick_orig,s,True)) for s in range(10)])
