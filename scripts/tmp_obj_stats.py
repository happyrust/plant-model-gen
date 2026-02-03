import sys
p = sys.argv[1]
vs=[]
fs=[]
with open(p,'r',encoding='utf-8',errors='ignore') as f:
    for line in f:
        if line.startswith('v '):
            _,x,y,z=line.split()[:4]
            vs.append((float(x),float(y),float(z)))
        elif line.startswith('f '):
            parts=line.split()[1:]
            fs.append([int(s.split('/')[0]) for s in parts])

mn=[float('inf')]*3
mx=[-float('inf')]*3
for x,y,z in vs:
    mn[0]=min(mn[0],x); mn[1]=min(mn[1],y); mn[2]=min(mn[2],z)
    mx[0]=max(mx[0],x); mx[1]=max(mx[1],y); mx[2]=max(mx[2],z)

print('verts',len(vs),'faces',len(fs))
print('bbox_min',mn)
print('bbox_max',mx)
print('bbox_size',[mx[i]-mn[i] for i in range(3)])

# vertex adjacency from faces
adj=[set() for _ in range(len(vs)+1)]
for tri in fs:
    if len(tri)<3:
        continue
    a,b,c=tri[0],tri[1],tri[2]
    adj[a].update([b,c]); adj[b].update([a,c]); adj[c].update([a,b])

seen=set(); comps=[]
for i in range(1,len(vs)+1):
    if i in seen:
        continue
    stack=[i]; seen.add(i)
    cnt=0
    while stack:
        u=stack.pop(); cnt+=1
        for v in adj[u]:
            if v not in seen:
                seen.add(v); stack.append(v)
    comps.append(cnt)
comps.sort(reverse=True)
print('vertex_components',len(comps),'largest',comps[:10])
