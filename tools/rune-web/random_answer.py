"""生成四位符文答案（允许重复，无第三方依赖）。

python random_answer.py
python random_answer.py --count 20
python random_answer.py --count 20 --seed 42 --json
"""
import argparse
import json
import random
import secrets

SYMBOLS = '空火水风土雷'

def generate(count=1, seed=None):
    rng = secrets.SystemRandom() if seed is None else random.Random(seed)
    return [''.join(rng.choice(SYMBOLS) for _ in range(4)) for _ in range(count)]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=1, help='生成数量，默认 1')
    parser.add_argument('--seed', type=int, help='可选随机种子，用于复现测试')
    parser.add_argument('--json', action='store_true', help='输出 JSON 数组')
    args = parser.parse_args()
    if args.count < 1:
        parser.error('--count 必须大于 0')
    answers = generate(args.count, args.seed)
    print(json.dumps(answers, ensure_ascii=False) if args.json else '\n'.join(answers))

if __name__ == '__main__':
    main()
