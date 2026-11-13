const symbols = '空火水风土雷';
const codes = Array.from({length:1296},(_,n)=>[Math.floor(n/216),Math.floor(n/36)%6,Math.floor(n/6)%6,n%6]);
// 判色规则固定为按数量匹配（count）：游戏按剩余数量消耗式匹配黄色
// （实拍样张 雷风风风/黄绿黄红 在 presence 下无解，v1.2.0 起删除 presence 分支）
function feedback(g,s){
  let colors=[0,0,0,0], remaining=[0,0,0,0,0,0];
  for(let i=0;i<4;i++) if(g[i]!==s[i]) remaining[s[i]]++;
  for(let i=0;i<4;i++){
    if(g[i]===s[i]) colors[i]=2;
    else if(remaining[g[i]]){colors[i]=1;remaining[g[i]]--;}
  }
  return colors.reduce((n,c,i)=>n+c*3**i,0);
}
function createSolver(){
  const matrix=codes.map(g=>Uint8Array.from(codes,s=>feedback(g,s)));
  // 候选优先策略（v1.2.9，用户偏好「搏直接命中」）：只在可能是答案的组合里选，
  // 同为候选时取最坏情况下剩余可能最少的（能区分剩局的候选天然胜出，且本身
  // 就有机会一发命中）；区分不了就按序猜，靠命中赌运气，不再给「探针步」。
  // 首猜固定 空火水风（全空间最优开局，本身也是候选）。
  function choose(cs,turns){
    if(!cs.length||turns<1)return null;
    if(cs.length===1)return cs[0];
    if(cs.length===1296)return 51;
    let best=-1,worst=0,sumsq=0;
    for(const gi of cs){
      const buckets=new Map();
      for(const si of cs){const r=matrix[gi][si];if(r===80)continue;const b=buckets.get(r);b?b.push(si):buckets.set(r,[si]);}
      const groups=[...buckets.values()];
      const l=Math.max(0,...groups.map(b=>b.length));
      const s=groups.reduce((n,b)=>n+b.length*b.length,0);
      if(best<0||l<worst||(l===worst&&s<sumsq)){best=gi;worst=l;sumsq=s;}
    }
    return best;
  }
  return {matrix,choose};
}
export { symbols, codes, feedback, createSolver };

