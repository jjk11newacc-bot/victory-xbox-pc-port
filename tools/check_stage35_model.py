"""Independent byte-level and lifetime models, not a Rust/GPU execution test."""
import random, hashlib
R=random.Random(35)
PAGE=16  # scaled-down block for exhaustive byte comparisons
BULK=4*PAGE

def exact_refresh(staging, known, guest, start, end, failed):
    dirty=[]
    at=start
    while at<end:
        nxt=min((at//BULK+1)*BULK,end)
        pages=range(at//PAGE,(nxt-1)//PAGE+1)
        if not any(known[p] for p in pages) or failed:
            staging[at:nxt]=guest[at:nxt];dirty.append((at,nxt))
        else:
            for p in pages:
                lo=max(at,p*PAGE);hi=min(nxt,(p+1)*PAGE)
                if not known[p] or staging[lo:hi]!=guest[lo:hi]:
                    staging[lo:hi]=guest[lo:hi];dirty.append((lo,hi))
        for p in pages:
            if p*PAGE>=at and min((p+1)*PAGE,len(guest))<=nxt:known[p]=True
        at=nxt
    return dirty

checks=0
for _ in range(2000):
    size=R.randrange(1,400)
    guest=bytearray(R.randbytes(size));staging=bytearray(R.randbytes(size));reference=staging[:]
    gpu=bytearray(R.randbytes(size));ref_gpu=gpu[:]
    known=[False]*((size+PAGE-1)//PAGE)
    pending=[];legacy=[]
    for step in range(50):
        for _ in range(R.randrange(5)):
            guest[R.randrange(size)]=R.randrange(256)
        a=R.randrange(size);b=R.randrange(a+1,size+1)
        # failed reads model the same legacy bytes (unmapped regions zero-filled)
        pending.extend(exact_refresh(staging,known,guest,a,b,R.randrange(13)==0))
        reference[a:b]=guest[a:b];legacy.append((a,b))
        assert staging==reference
        if R.randrange(3)==0 or step==49:
            for lo,hi in pending:gpu[lo:hi]=staging[lo:hi]
            for lo,hi in legacy:ref_gpu[lo:hi]=reference[lo:hi]
            assert gpu==ref_gpu
            pending.clear();legacy.clear()
        checks+=1
print(f'exact unwatched uploads: {checks} byte/state comparisons passed')

# Keep legacy first-batch duplicates, exact range order and owner identity.
for _ in range(3000):
    old={};new={};seen={}
    for _ in range(50):
        k=R.randrange(7);rr=[(a:=R.randrange(40),a+R.randrange(1,25)) for _ in range(R.randrange(8))]
        if k not in old:old[k]=rr[:]
        else:
            for x in rr:
                if x not in old[k]:old[k].append(x)
        if k not in new:new[k]=rr[:];seen[k]=set(rr)
        else:
            for x in rr:
                if x not in seen[k]:seen[k].add(x);new[k].append(x)
        assert list(old.items())==list(new.items())
print('indexed declaration order: 150000 events passed')

# Independently reconstruct revisions using raw-page fingerprints, including
# unsubmitted revisions, repeat records, and late completion. Model assumes
# collision-free hashes for generated data, actual GPU uses BLAKE3.
for _ in range(2000):
    n=R.randrange(4,400)*4
    tables=R.randbytes(R.randrange(1,30)*4)
    cpu=bytearray(R.randbytes(n));device=None;submitted=False
    baseline=None;dirty=None
    def hashes(raw):return [hashlib.sha256(raw[i:i+PAGE]).digest() for i in range(0,len(raw),PAGE)]
    old=hashes(cpu)
    for _ in range(30):
        new=cpu[:]
        for _ in range(R.randrange(4)):new[R.randrange(n)]=R.randrange(256)
        newhash=hashes(new)
        if submitted:
            assert device==bytes(cpu)+tables
            dirty=[(i*PAGE,min((i+1)*PAGE,n)) for i,(a,b) in enumerate(zip(old,newhash)) if a!=b]
        else:dirty=None
        if dirty is None:cpu=new[:]
        else:
            for a,b in dirty:cpu[a:b]=new[a:b]
        assert cpu==new
        old=newhash;submitted=False
        # A canceled prelude does not initialize the next GPU revision.
        if R.randrange(3):
            for _ in range(R.randrange(1,4)):
                if dirty is None:device=bytes(cpu)+tables
                else:
                    patched=bytearray(device)
                    for a,b in dirty:patched[a:b]=cpu[a:b]
                    device=bytes(patched)
                assert device==bytes(new)+tables
            submitted=True
print('dirty GPU texture uploads: 60000 revision transitions passed')

# Compact partial footprint placements preserve original subresource mapping.
for _ in range(10000):
    count=R.randrange(2,16);offset=0;parts=[]
    for mip in range(count):
        offset=(offset+511)//512*512
        rows=R.randrange(1,300);pitch=R.randrange(1,33)*256
        parts.append((offset,rows*pitch));offset+=rows*pitch
    first=R.randrange(1,count);base=parts[first][0];out=offset-base
    prior_end=0
    for pos,n in parts[first:]:
        rebased=pos-base
        assert rebased%512==0 and prior_end<=rebased<rebased+n<=out
        assert rebased+base==pos
        prior_end=rebased+n
    assert prior_end==out
print('partial GPU output footprints: 10000 plans passed')
