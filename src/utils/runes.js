const symbols = '空火水风土雷';
const codes = Array.from({length:1296},(_,n)=>[Math.floor(n/216),Math.floor(n/36)%6,Math.floor(n/6)%6,n%6]);
function feedback(g,s,mode){
  let colors=[0,0,0,0], remaining=[0,0,0,0,0,0];
  for(let i=0;i<4;i++) if(g[i]!==s[i]) remaining[s[i]]++;
  for(let i=0;i<4;i++){
    if(g[i]===s[i]) colors[i]=2;
    else if(mode==='presence' && s.includes(g[i])) colors[i]=1;
    else if(mode==='count' && remaining[g[i]]){colors[i]=1;remaining[g[i]]--;}
  }
  return colors.reduce((n,c,i)=>n+c*3**i,0);
}
function createSolver(mode){
  const matrix=codes.map(g=>Uint8Array.from(codes,s=>feedback(g,s,mode))), cache=new Map();
  function choose(cs,turns){
    if(!cs.length||turns<1)return null;
    if(cs.length===1)return cs[0];
    if(turns===1)return null;
    const key=turns+':'+cs.join(',');if(cache.has(key))return cache.get(key);
    const options=[],set=new Set(cs),guesses=cs.length===1296?[51]:codes.keys();
    for(const gi of guesses){
      const buckets=new Map();
      for(const si of cs){const r=matrix[gi][si];if(r===80)continue;if(!buckets.has(r))buckets.set(r,[]);buckets.get(r).push(si);}
      const groups=[...buckets.values()],largest=Math.max(0,...groups.map(b=>b.length));
      if(largest===cs.length || (turns===2&&largest>1))continue;
      options.push({gi,groups,score:[largest,groups.reduce((n,b)=>n+b.length*b.length,0),set.has(gi)?0:1]});
    }
    options.sort((a,b)=>a.score[0]-b.score[0]||a.score[1]-b.score[1]||a.score[2]-b.score[2]||a.gi-b.gi);
    for(const o of options)if(o.groups.sort((a,b)=>b.length-a.length).every(b=>choose(b,turns-1)!==null)){cache.set(key,o.gi);return o.gi;}
    cache.set(key,null);return null;
  }
  return {matrix,choose};
}
export { symbols, codes, feedback, createSolver };

