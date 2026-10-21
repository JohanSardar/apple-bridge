export function bubbleSilhouette(width:number,height:number){
    const radius=Math.min(18,width/2,height/2);
    return `M${radius} 0H${width-radius}Q${width} 0 ${width} ${radius}V${height-12}C${width} ${height-5} ${width+1} ${height-2} ${width+5} ${height}C${width-1} ${height} ${width-6} ${height-1} ${width-9} ${height-4}Q${width-12} ${height} ${width-radius} ${height}H${radius}Q0 ${height} 0 ${height-radius}V${radius}Q0 0 ${radius} 0Z`;
}
