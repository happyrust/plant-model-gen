import sys, math
p=sys.argv[1]
xs=[]
with open(p,'r',encoding='utf-8',errors='ignore') as f:
    for line in f:
        if line.startswith('v '):
            _,x,y,z=line.split()[:4]
            xs.append(float(x))

mn=min(xs); mx=max(xs)
center=(mn+mx)/2
print('min',mn,'max',mx,'center',center,'span',mx-mn)
for w in [1,2,5,10,15,20]:
    c=sum(1 for x in xs if abs(x-center)<=w)
    print('slab_half_width',w,'count',c)
# nearest offsets
min_abs=min(abs(x-center) for x in xs)
print('min_abs_offset',min_abs)
near=sorted({round(x-center,6) for x in xs if abs(x-center)<=20})
print('unique_offsets_near_center',near)
