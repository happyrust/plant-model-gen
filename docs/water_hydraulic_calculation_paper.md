# 小型排涝明渠工程中设计流量与断面尺寸的联合水利计算方法研究

## 摘要

小型排涝、灌排结合及厂区雨洪外排工程常采用“设计暴雨-产汇流-渠道过流能力校核”的水利计算流程。由于该类工程通常缺少长序列实测流量资料，工程设计需要在规范约束、经验方法和水力学模型之间取得平衡。本文以小流域排涝明渠为对象，构建设计流量推求、梯形明渠均匀流计算、断面安全校核和参数敏感性分析的联合计算框架。方法上，设计洪峰流量采用暴雨强度、径流系数和汇水面积推求；渠道断面采用 Manning 公式求解正常水深，并以流速、Froude 数、超高和糙率敏感性作为复核指标。算例表明，当汇水面积为 2.0 km2、径流系数为 0.45、设计暴雨强度为 60 mm/h 时，设计流量为 15.012 m3/s；取底宽 3.0 m、边坡系数 1.5、糙率 0.030、纵坡 0.001 的梯形渠道，正常水深约为 2.064 m，平均流速约为 1.193 m/s，Froude 数约为 0.326，水流状态为缓流。研究认为，小型水利工程的计算质量不只取决于公式本身，还取决于资料复核、糙率取值、边界条件、超高控制和不确定性表达。本文提出的流程可作为初步设计阶段的技术校核模板，但施工图设计仍需结合实测地形、水文频率分析、排水受纳水位和现行规范进行复核。

**关键词：** 水利计算；设计流量；Manning 公式；明渠均匀流；排涝渠道；参数敏感性

## Abstract

Small drainage and irrigation-drainage channels are commonly designed through an integrated workflow linking design rainfall, runoff estimation, and hydraulic capacity verification. Because long-term discharge records are often unavailable for small catchments, engineering design must balance code-based requirements, empirical hydrologic methods, and hydraulic modeling. This paper develops an integrated calculation framework for design discharge estimation, trapezoidal open-channel normal-depth computation, safety checks, and parameter sensitivity analysis. The design peak discharge is estimated from rainfall intensity, runoff coefficient, and drainage area; the channel section is verified using Manning's equation, velocity, Froude number, freeboard, and roughness sensitivity. In the illustrative case, a drainage area of 2.0 km2, runoff coefficient of 0.45, and rainfall intensity of 60 mm/h produce a design discharge of 15.012 m3/s. A trapezoidal channel with a 3.0 m bottom width, 1.5 side slope, Manning's n of 0.030, and longitudinal slope of 0.001 yields a normal depth of approximately 2.064 m, mean velocity of 1.193 m/s, and Froude number of 0.326, indicating subcritical flow. The study concludes that calculation quality depends not only on formulas, but also on data verification, roughness selection, boundary conditions, freeboard control, and uncertainty reporting.

**Keywords:** hydraulic calculation; design discharge; Manning equation; open-channel flow; drainage channel; sensitivity analysis

## 1. 引言

水利计算是水利工程规划、初步设计、运行复核和风险评估的基础环节。对大中型水利水电工程而言，水文计算通常需要完整的资料搜集、径流分析、设计洪水计算、水位流量关系拟定等内容；中国水利行业标准 SL/T 278-2020 已将这些内容作为水文计算的重要组成部分，并明确其适用于大中型水利水电工程的水文计算。对小型排涝明渠和厂区外排工程而言，资料条件往往弱于大中型流域，设计人员更常使用经验暴雨公式、推理公式或降雨径流模型估计设计流量，再通过明渠水力学公式校核断面过流能力。

国际工程实践也呈现类似逻辑。HEC-HMS 将流域概化为子流域、河段、汇流点、水库等水文单元，可进行降雨损失、产流转换、河道汇流、频率分析和不确定性分析；HEC-RAS 则将 Manning 公式、糙率、断面形态和水面线计算用于渠道及河道水力分析。二者共同说明，可靠的水利计算不应把水文学和水力学割裂处理，而应形成从降雨到流量、从流量到断面、从断面到安全裕度的闭合流程。

本文针对小型排涝明渠初步设计阶段，提出一套简化但可追溯的联合计算方法。研究目标包括：第一，明确设计流量推求与明渠过流能力计算之间的数据接口；第二，给出梯形渠道正常水深、流速和水流状态的计算步骤；第三，通过算例说明参数取值对工程结论的影响；第四，总结该类计算在实际设计中的适用边界。

## 2. 资料与方法

### 2.1 设计流量推求

当缺少实测洪峰流量资料且汇水面积较小时，可在地区暴雨强度公式或设计暴雨成果基础上，采用推理公式估计设计洪峰流量：

```text
Qp = 0.278 * C * i * A
```

式中，`Qp` 为设计洪峰流量，单位为 m3/s；`C` 为综合径流系数；`i` 为设计历时内平均降雨强度，单位为 mm/h；`A` 为汇水面积，单位为 km2；0.278 为单位换算系数。该式适合小流域和城市化汇水区的初步估算，关键假设是降雨历时与汇流时间匹配，且汇水区降雨近似均匀。若工程等级较高、汇水面积较大或存在水库、闸站、分洪区等调蓄单元，则应转向频率分析、单位线法、产汇流模型或连续模拟方法。HEC-HMS 的能力清单显示，其支持多种降雨损失、产流转换、河道演算、频率分析和 Monte Carlo 不确定性分析方法，可作为复杂工况下的补充建模工具。

### 2.2 梯形明渠均匀流计算

对规则梯形渠道，断面面积、湿周和水力半径分别为：

```text
A = b*y + m*y^2
P = b + 2*y*sqrt(1 + m^2)
R = A / P
```

式中，`b` 为渠底宽度，`y` 为水深，`m` 为单侧边坡系数，`A` 为过水面积，`P` 为湿周，`R` 为水力半径。均匀流过流能力采用 Manning 公式：

```text
Q = (1/n) * A * R^(2/3) * S^(1/2)
```

式中，`n` 为 Manning 糙率系数，`S` 为能量坡度。HEC-RAS 官方手册说明，均匀流计算可由水深、宽度、坡度、流量和糙率五个参数中的四个求解第五个参数；对梯形断面，可将面积和水力半径表达为水深、底宽和边坡的函数。本文据此采用二分法求解满足 `Q(y)=Qp` 的正常水深。

### 2.3 校核指标

断面尺寸不能仅以“过流能力等于设计流量”为唯一判据，还应至少复核以下指标：

1. 平均流速 `V = Q/A`。流速过大可能引起冲刷，过小则可能导致淤积。
2. Froude 数 `Fr = V / sqrt(g*D)`，其中 `D=A/T` 为水力深度，`T=b+2*m*y` 为水面宽。`Fr<1` 表示缓流，`Fr>1` 表示急流。
3. 渠顶超高。设计水深以上应保留足够超高，以吸收糙率误差、局部壅水、施工偏差和水面波动。
4. 糙率敏感性。HEC-RAS 粗糙系数说明指出，Manning n 值受河床材料、断面不规则性、植被、障碍物、淤积冲刷和水位流量阶段变化影响，实际取值需要工程判断，并建议在风险评估中进行上下浮动敏感性分析。

## 3. 算例

设某厂区及周边小流域需新建排涝明渠，汇水面积 `A=2.0 km2`，综合径流系数 `C=0.45`，设计暴雨强度 `i=60 mm/h`。由推理公式得：

```text
Qp = 0.278 * 0.45 * 60 * 2.0 = 15.012 m3/s
```

拟采用梯形土渠或衬砌渠道，初选参数为：渠底宽 `b=3.0 m`，边坡系数 `m=1.5`，Manning 糙率 `n=0.030`，渠道纵坡 `S=0.001`。代入 Manning 公式并用二分法求解正常水深，可得：

```text
y = 2.064 m
A = 12.578 m2
P = 10.440 m
R = 1.205 m
V = 1.193 m/s
Fr = 0.326
```

计算结果显示，该断面在所取糙率和坡度下能够通过 15.012 m3/s 的设计流量，且 `Fr<1`，属于缓流。若按运行水位以上设置 0.4-0.6 m 的初步超高，渠深可取不小于 2.5-2.7 m。最终渠深仍需结合衬砌形式、管理道路、交叉建筑物、下游控制水位和当地规范限值确定。

## 4. 敏感性分析与讨论

### 4.1 糙率对水深的影响

Manning 公式中流量与 `1/n` 成正比。在其他条件不变时，糙率取值偏大将降低过流能力并抬高正常水深。对天然土渠、植被渠道或维护条件不稳定的排涝沟，糙率的不确定性通常大于几何尺寸测量误差。若将 `n=0.030` 上浮 20% 至 0.036，则原断面过流能力下降，需增加水深或底宽才能维持设计流量。工程设计中宜将糙率敏感性作为成果表的一部分，而不是只给出单一水深。

### 4.2 纵坡与受纳水位的影响

均匀流假设要求渠道坡度、糙率和断面沿程近似稳定，且下游水位不形成明显控制。实际排涝明渠常受河道顶托、涵闸控制、泵站运行水位和交叉建筑物局部水头损失影响。若下游水位高于正常水深对应水面线，渠道将进入非均匀流或壅水状态，单纯均匀流计算会低估水深。因此，初步设计可用 Manning 公式确定断面量级，重要节点和下游控制工况应使用水面线计算或一维非恒定模型复核。

### 4.3 设计流量的不确定性

设计流量中的暴雨强度、径流系数和汇流时间均存在不确定性。径流系数受地面硬化率、土壤入渗、前期含水量、排水管网连通性影响；设计暴雨强度受重现期、历时和地区暴雨公式影响。HEC-HMS 支持频率分析和不确定性分析，这提示小型工程也应至少进行情景复核，例如采用低、中、高三组径流系数或暴雨强度组合，观察渠底宽、水深和超高是否仍满足安全要求。

## 5. 工程应用建议

第一，计算书应把水文参数、水力参数和几何参数分表列出，注明来源和适用条件。对引用现行规范、地方暴雨强度公式或测站资料的参数，应保留版本号、发布日期和资料年限。

第二，初步设计阶段可采用推理公式与 Manning 公式快速形成断面方案，但不宜跳过下游水位、局部损失和交叉建筑物复核。涵洞、跌水、闸门、桥梁等构筑物会改变局部能量线，必要时应单独计算。

第三，成果表达应从“单点确定值”转向“设计值加敏感性范围”。对于小型排涝工程，建议至少给出 `n` 上下浮动 20%、设计流量上浮 10%-20% 时的水深变化，并据此确定超高。

第四，运行管理条件应进入设计假设。若渠道后期可能出现植被生长、淤积或垃圾阻水，设计糙率和超高应适当保守，并在运行期设定清淤维护标准。

## 6. 结论

本文提出了小型排涝明渠中设计流量与断面尺寸的联合水利计算框架。算例显示，在汇水面积 2.0 km2、径流系数 0.45、设计暴雨强度 60 mm/h 的条件下，设计流量为 15.012 m3/s；采用底宽 3.0 m、边坡 1.5、糙率 0.030、纵坡 0.001 的梯形渠道时，正常水深约 2.064 m，平均流速约 1.193 m/s，水流为缓流。该结果说明，Manning 均匀流公式能够为初步断面设计提供清晰、可复核的计算路径。

但本文算例也表明，水利计算的可靠性主要受参数取值和边界条件控制。实际工程中，应将现行规范、实测地形、下游水位、局部建筑物损失和运行维护条件纳入复核，并通过敏感性分析表达不确定性。对工程等级较高或调蓄关系复杂的项目，建议采用 HEC-HMS、HEC-RAS 或同类模型开展水文-水力耦合校核。

## 数据可得性声明

本文为方法研究与示例计算，未使用专有实测数据。算例参数为假定值，仅用于说明计算流程。

## 伦理声明

本文不涉及人体或动物实验，也不涉及个人敏感信息。

## 作者贡献

概念化、方法设计、计算、初稿写作与修订均由作者完成。正式投稿前可按 CRediT 角色重新拆分作者贡献。

## 利益冲突声明

作者声明不存在已知利益冲突。

## 基金声明

本文未获得特定基金资助。若用于课程论文、项目报告或投稿，应按实际资助情况补充。

## AI 使用声明

本文初稿在 AI 写作工具辅助下形成，作者需对公式、参数、规范版本、参考文献和工程结论进行人工复核。该文本不应直接替代注册工程师出具的设计计算书。

## 参考文献

Chow, V. T. (1959). *Open-channel hydraulics*. McGraw-Hill. https://archive.org/details/openchannelhydra0000chow

Chow, V. T., Maidment, D. R., & Mays, L. W. (1988). *Applied hydrology*. McGraw-Hill. https://www.scirp.org/reference/referencespapers?referenceid=1165939

Hydrologic Engineering Center. (2026a). *HEC-HMS User's Manual* (Version 4.14 online documentation). U.S. Army Corps of Engineers. https://www.hec.usace.army.mil/confluence/hmsdocs/hmsum/latest

Hydrologic Engineering Center. (2026b). *HEC-RAS Hydraulic Reference Manual* (Version 7.0 online documentation). U.S. Army Corps of Engineers. https://www.hec.usace.army.mil/confluence/rasdocs/ras1dtechref/latest

中华人民共和国水利部. (2015). *SL/T 104-2015 水利工程水利计算规范*. 全国标准信息公共服务平台. https://std.samr.gov.cn/hb/search/stdHBDetailed?id=8B1827F14CC9BB19E05397BE0A0AB44A

中华人民共和国水利部. (2020). *SL/T 278-2020 水利水电工程水文计算规范*. 中国标准化研究院新闻资料. https://www.cnis.ac.cn/gnbzh/gndt/202007/t20200729_50141.html
